//! Reading the web, safely. The approach is ported from Devsper's memory service (svc/memory/src/ingest_sources.rs):
//! public addresses only (checked after DNS, and again on every redirect), a response size cap,
//! Readability for the main text, and a headless Chrome render for pages that are empty until JavaScript runs.
//! What comes back is data: nothing here ever treats page text as instructions.
use dom_smoothie::Readability;
use scraper::{Html, Selector};
use std::io::Read;
use std::net::{IpAddr, Ipv6Addr, ToSocketAddrs};
use std::time::Duration;

const MAX_BYTES: u64 = 10 * 1024 * 1024; // an uncapped read is an easy OOM
const MAX_REDIRECTS: u8 = 5;
/// Less text than this from a plain fetch means "probably a JavaScript shell": worth a headless render.
const SPA_TEXT_THRESHOLD: usize = 200;
const RENDER_TIMEOUT: Duration = Duration::from_secs(25);
const HYDRATE_DELAY: Duration = Duration::from_millis(1200);
const USER_AGENT: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.0 Safari/605.1.15 Sunday/1.0";

pub struct Page {
    pub url: String,
    pub title: String,
    pub text: String,
    /// True if a headless browser was needed to see the content.
    pub rendered: bool,
}

fn is_public_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => !(v4.is_private() || v4.is_loopback() || v4.is_link_local() || v4.is_unspecified() || v4.is_multicast() || v4.is_broadcast() || v4.is_documentation()),
        IpAddr::V6(v6) => !(v6.is_loopback() || v6.is_unspecified() || v6.is_multicast() || unique_local(&v6) || link_local(&v6)),
    }
}
fn unique_local(v6: &Ipv6Addr) -> bool {
    (v6.segments()[0] & 0xfe00) == 0xfc00
}
fn link_local(v6: &Ipv6Addr) -> bool {
    (v6.segments()[0] & 0xffc0) == 0xfe80
}

/// http(s) only, and the host must resolve to public addresses only (resolving, not string-matching,
/// is what catches a hostname that points at an internal address).
pub fn validate_target(url: &url::Url) -> Result<(), String> {
    if !matches!(url.scheme(), "http" | "https") {
        return Err(format!("unsupported url scheme '{}'", url.scheme()));
    }
    let host = url.host_str().ok_or("url has no host")?;
    if let Ok(ip) = host.trim_matches(['[', ']']).parse::<IpAddr>() {
        return if is_public_ip(ip) { Ok(()) } else { Err(format!("refusing to fetch a private or internal address: {ip}")) };
    }
    let addrs: Vec<_> = (host, url.port_or_known_default().unwrap_or(80)).to_socket_addrs().map_err(|e| format!("dns lookup failed for {host}: {e}"))?.collect();
    if addrs.is_empty() {
        return Err(format!("dns lookup for {host} returned no addresses"));
    }
    match addrs.iter().find(|a| !is_public_ip(a.ip())) {
        Some(a) => Err(format!("refusing to fetch {host}: it resolves to a private or internal address ({})", a.ip())),
        None => Ok(()),
    }
}

/// The page's readable text and title. Readability isolates the article; for pages that aren't articles
/// (indexes, landing pages) it falls back to all visible text minus script/style/nav.
pub fn extract(html: &str, url: &str) -> (String, String) {
    if let Ok(a) = Readability::new(html, Some(url), None).and_then(|mut r| r.parse()) {
        let text = a.text_content.to_string();
        if !text.trim().is_empty() {
            return (a.title.to_string(), collapse(&text));
        }
    }
    let doc = Html::parse_document(html);
    let title = Selector::parse("title").ok().and_then(|s| doc.select(&s).next()).map(|t| t.text().collect::<String>()).unwrap_or_default();
    (title.trim().to_string(), visible_text(&doc))
}

fn collapse(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn visible_text(doc: &Html) -> String {
    const SKIP: [&str; 6] = ["script", "style", "nav", "footer", "header", "noscript"];
    let mut out = String::new();
    for node in doc.tree.nodes() {
        let scraper::node::Node::Text(t) = node.value() else { continue };
        if !node.ancestors().any(|a| a.value().as_element().is_some_and(|e| SKIP.contains(&e.name()))) {
            out.push_str(t);
            out.push(' ');
        }
    }
    collapse(&out)
}

/// Follows redirects itself so every hop gets the address check, and reads at most MAX_BYTES.
fn get(url: &url::Url) -> Result<(String, String), String> {
    let agent = ureq::AgentBuilder::new().redirects(0).timeout(Duration::from_secs(20)).user_agent(USER_AGENT).build();
    let mut current = url.clone();
    for _ in 0..=MAX_REDIRECTS {
        validate_target(&current)?;
        let resp = match agent.get(current.as_str()).call() {
            Ok(r) => r,
            Err(ureq::Error::Status(c, _)) => return Err(format!("the site answered {c}")),
            Err(e) => return Err(e.to_string()),
        };
        if (300..400).contains(&resp.status()) {
            let loc = resp.header("location").ok_or("redirect without a location")?;
            current = current.join(loc).map_err(|e| e.to_string())?;
            continue;
        }
        let html = resp.header("content-type").map_or(true, |c| c.contains("html") || c.contains("text"));
        if !html {
            return Err("that address isn't a web page".into());
        }
        let mut body = String::new();
        resp.into_reader().take(MAX_BYTES + 1).read_to_string(&mut body).map_err(|e| e.to_string())?;
        if body.len() as u64 > MAX_BYTES {
            return Err("the page is too large".into());
        }
        return Ok((current.to_string(), body));
    }
    Err("too many redirects".into())
}

/// A Chromium-family browser to drive headlessly: Chrome first, then others.
pub fn chrome_path() -> Option<std::path::PathBuf> {
    const APPS: [&str; 6] = [
        "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
        "/Applications/Chromium.app/Contents/MacOS/Chromium",
        "/Applications/Brave Browser.app/Contents/MacOS/Brave Browser",
        "/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge",
        "/Applications/Vivaldi.app/Contents/MacOS/Vivaldi",
        "/usr/bin/chromium",
    ];
    std::env::var_os("SUNDAY_CHROME").map(std::path::PathBuf::from).filter(|p| p.exists()).or_else(|| APPS.iter().map(std::path::PathBuf::from).find(|p| p.exists()))
}

/// The page's HTML after JavaScript has run. None if no browser is installed or anything fails or times out,
/// so a page that can't be rendered just keeps whatever the plain fetch found.
fn render(url: &str) -> Option<String> {
    use chromiumoxide::{Browser, BrowserConfig};
    use futures::StreamExt;
    let exe = chrome_path()?;
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().ok()?;
    rt.block_on(async {
        tokio::time::timeout(RENDER_TIMEOUT, async {
            let profile = std::env::temp_dir().join(format!("sunday-chrome-{}", std::process::id()));
            let cfg = BrowserConfig::builder().chrome_executable(exe).no_sandbox().user_data_dir(&profile).build().ok()?;
            let (mut browser, mut handler) = Browser::launch(cfg).await.ok()?;
            // The handler drives the connection and must be polled for the browser's whole life.
            let driver = tokio::spawn(async move { while handler.next().await.is_some() {} });
            let html = async {
                let page = browser.new_page(url).await.ok()?;
                page.wait_for_navigation().await.ok()?;
                tokio::time::sleep(HYDRATE_DELAY).await; // there's no universal "finished rendering" signal
                let html = page.content().await.ok();
                let _ = page.close().await;
                html
            }
            .await;
            let _ = browser.close().await;
            let _ = browser.wait().await;
            driver.abort();
            let _ = std::fs::remove_dir_all(&profile);
            html
        })
        .await
        .ok()
        .flatten()
    })
}

/// Reads one web page. A headless browser is used only if the plain fetch finds almost no text.
pub fn fetch(address: &str) -> Result<Page, String> {
    let url = url::Url::parse(address.trim()).map_err(|_| "that doesn't look like a web address".to_string())?;
    let (final_url, html) = get(&url)?;
    let (mut title, mut text) = extract(&html, &final_url);
    let mut rendered = false;
    if text.len() < SPA_TEXT_THRESHOLD {
        if let Some(h) = render(&final_url) {
            let (t2, x2) = extract(&h, &final_url);
            if x2.len() > text.len() {
                (title, text, rendered) = (t2, x2, true);
            }
        }
    }
    if text.trim().is_empty() {
        return Err("I couldn't find any readable text on that page".into());
    }
    Ok(Page { url: final_url, title: title.trim().to_string(), text, rendered })
}

/// The paragraphs of `text` most relevant to `q` (by shared words), kept in reading order, up to `max` chars.
/// Readability output is one long line, so this works on sentence-aligned pieces of about 300 characters.
pub fn snippet(text: &str, q: &str, max: usize) -> String {
    let terms = crate::notes::terms(q);
    let (mut pieces, mut cur): (Vec<String>, String) = (Vec::new(), String::new());
    for sentence in text.split_inclusive(['.', '!', '?']) {
        cur.push_str(sentence);
        if cur.len() >= 300 {
            pieces.push(std::mem::take(&mut cur));
        }
    }
    if !cur.trim().is_empty() {
        pieces.push(cur);
    }
    let mut ranked: Vec<(usize, usize)> = pieces
        .iter()
        .enumerate()
        .map(|(i, p)| {
            let l = p.to_lowercase();
            (terms.iter().filter(|t| l.contains(t.as_str())).count(), i)
        })
        .collect();
    // Most relevant first; ties go to the earlier piece (intros usually say what a page is about).
    ranked.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    let (mut keep, mut used) = (Vec::new(), 0);
    for (_, i) in ranked {
        let len = pieces[i].len().min(max);
        if used + len > max && !keep.is_empty() {
            continue;
        }
        used += len;
        keep.push(i);
    }
    keep.sort_unstable();
    let joined = keep.iter().map(|&i| pieces[i].trim().to_string()).collect::<Vec<_>>().join(" ... ");
    joined.chars().take(max).collect()
}

/// The address of the page open in the front browser (macOS, via AppleScript: the first use asks for permission).
pub fn browser_url(front_app: &str) -> Result<String, String> {
    const CHROMIUM: [&str; 7] = ["Google Chrome", "Brave Browser", "Microsoft Edge", "Chromium", "Vivaldi", "Arc", "Dia"];
    let script = match front_app {
        "Safari" => r#"tell application "Safari" to get URL of front document"#.to_string(),
        a if CHROMIUM.contains(&a) => format!(r#"tell application "{a}" to get URL of active tab of front window"#),
        a if a.eq_ignore_ascii_case("sunday") => return Err("Switch to the browser tab first and ask by voice, or paste the link".into()),
        a => return Err(format!("{a} isn't a browser I can read an address from")),
    };
    let out = std::process::Command::new("osascript").arg("-e").arg(&script).output().map_err(|e| e.to_string())?;
    let url = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if out.status.success() && url.starts_with("http") {
        Ok(url)
    } else {
        Err(format!("I couldn't read the address from {front_app}. Allow this app to control it in System Settings, Privacy and Security, Automation"))
    }
}

/// A web search (DuckDuckGo's HTML page): result addresses and titles, best first.
pub fn search(query: &str, n: usize) -> Result<Vec<(String, String)>, String> {
    let url = url::Url::parse_with_params("https://html.duckduckgo.com/html/", [("q", query)]).map_err(|e| e.to_string())?;
    let (_, html) = get(&url)?;
    let hits = parse_results(&html);
    if hits.is_empty() {
        return Err("the search came back empty".into());
    }
    Ok(hits.into_iter().take(n).collect())
}

/// Result links on a DuckDuckGo HTML results page. Links go through a redirector with the real
/// address in the `uddg` parameter.
pub fn parse_results(html: &str) -> Vec<(String, String)> {
    let doc = Html::parse_document(html);
    let sel = Selector::parse("a.result__a").expect("valid selector");
    doc.select(&sel)
        .filter_map(|a| {
            let href = a.value().attr("href")?;
            let full = if href.starts_with("//") { format!("https:{href}") } else { href.to_string() };
            let u = url::Url::parse(&full).ok()?;
            let real = u.query_pairs().find(|(k, _)| k == "uddg").map(|(_, v)| v.into_owned()).unwrap_or(full);
            real.starts_with("http").then(|| (real, collapse(&a.text().collect::<String>())))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refuses_private_and_odd_addresses() {
        let bad = ["http://127.0.0.1/", "http://10.0.0.5/x", "http://192.168.1.1/", "http://169.254.169.254/latest/meta-data", "http://[::1]/", "http://localhost:8080/", "file:///etc/passwd", "ftp://example.com/"];
        for b in bad {
            assert!(validate_target(&url::Url::parse(b).unwrap()).is_err(), "{b} should be refused");
        }
        assert!(validate_target(&url::Url::parse("http://93.184.215.14/").unwrap()).is_ok());
    }

    #[test]
    fn extracts_the_article_not_the_chrome() {
        let html = r#"<html><head><title>Deadlocks explained</title></head><body>
            <nav>Home Pricing Login</nav><article><h1>Deadlocks explained</h1>
            <p>A deadlock happens when processes wait on each other forever. It needs four conditions: mutual exclusion, hold and wait, no preemption and circular wait. We cover each of them in turn with examples from operating systems.</p>
            <p>The Banker's algorithm avoids deadlock by only granting a request when the system stays in a safe state, which is checked before every allocation.</p></article>
            <footer>Copyright</footer><script>track()</script></body></html>"#;
        let (title, text) = extract(html, "https://example.com/os");
        assert!(title.contains("Deadlock"), "{title}");
        assert!(text.contains("circular wait") && !text.contains("track()") && !text.contains("Pricing"), "{text}");
        // Not an article at all: the plain fallback still finds the visible text.
        let (_, t) = extract("<html><body><nav>x</nav><div>Just a short landing page about graphs.</div><script>y()</script></body></html>", "https://e.com");
        assert!(t.contains("landing page") && !t.contains("y()"), "{t}");
    }

    #[test]
    fn parses_search_results() {
        let html = r#"<div class="result"><a class="result__a" href="//duckduckgo.com/l/?uddg=https%3A%2F%2Fen.wikipedia.org%2Fwiki%2FDeadlock&amp;rut=abc">Deadlock - Wikipedia</a></div>
            <div class="result"><a class="result__a" href="https://example.com/direct">Direct link</a></div>
            <div class="result"><a class="result__a" href="javascript:void(0)">Bad</a></div>"#;
        let r = parse_results(html);
        assert_eq!(r, vec![("https://en.wikipedia.org/wiki/Deadlock".to_string(), "Deadlock - Wikipedia".to_string()), ("https://example.com/direct".to_string(), "Direct link".to_string())]);
    }

    #[test]
    fn snippet_keeps_the_relevant_parts_in_order() {
        let filler = "This sentence is about something else entirely and goes on for a while to take up space. ".repeat(12);
        let text = format!("An intro about operating systems in general. {filler}The Banker's algorithm checks whether a state is safe before granting a request. {filler}Paging maps virtual addresses to frames. {filler}");
        let s = snippet(&text, "how does the banker's algorithm check safe states", 700);
        assert!(s.len() <= 700 && s.contains("Banker's algorithm checks"), "{s}");
        assert!(!s.contains("Paging"), "irrelevant parts are left out");
        assert_eq!(snippet("short text.", "anything", 700), "short text.");
        assert!(snippet("", "q", 100).is_empty());
        assert!(browser_url("Ghostty").unwrap_err().contains("isn't a browser"));
        assert!(browser_url("Sunday").unwrap_err().contains("paste the link"));
    }

    /// Needs the network (and Chrome for the last part); run by hand with SUNDAY_WEB_TEST=1.
    #[test]
    fn reads_real_pages() {
        if std::env::var("SUNDAY_WEB_TEST").is_err() {
            return;
        }
        let p = fetch("https://en.wikipedia.org/wiki/Deadlock_(computer_science)").expect("static page");
        println!("static: {:?} rendered={} {} chars: {}", p.title, p.rendered, p.text.len(), &p.text[..200.min(p.text.len())]);
        assert!(!p.rendered && p.text.len() > 1000);
        let hits = search("banker's algorithm deadlock avoidance", 4).expect("search");
        println!("search: {hits:#?}");
        assert!(!hits.is_empty());
        println!("chrome: {:?}", chrome_path());
        let html = render("https://example.com").expect("headless render");
        assert!(html.contains("Example Domain"), "{}", &html[..200.min(html.len())]);
    }
}
