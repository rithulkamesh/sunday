//! Imported documents: extract text, split it into chunks, and build search queries.
use std::path::Path;

const MIN_CHUNK: usize = 500;
const MAX_CHUNK: usize = 1200;

/// (title, text) for a PDF, Markdown or plain-text file. PDFs go through poppler's `pdftotext`.
pub fn extract(path: &Path) -> Result<(String, String), String> {
    let title = path.file_stem().map_or_else(|| "notes".into(), |s| s.to_string_lossy().into_owned());
    let ext = path.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
    let text = match ext.as_str() {
        "pdf" => {
            let out = std::process::Command::new("pdftotext").arg("-layout").arg(path).arg("-").output()
                .map_err(|_| "PDF import needs poppler: brew install poppler".to_string())?;
            if !out.status.success() {
                return Err(format!("couldn't read {}", path.display()));
            }
            String::from_utf8_lossy(&out.stdout).into_owned()
        }
        "md" | "markdown" | "txt" | "text" | "org" | "rst" => std::fs::read_to_string(path).map_err(|e| e.to_string())?,
        _ => return Err(format!("unsupported file type: .{ext} (use pdf, md or txt)")),
    };
    if text.trim().is_empty() {
        return Err(format!("{} has no text (a scanned PDF?)", path.display()));
    }
    Ok((title, text))
}

/// Paragraph-based chunks of roughly MIN_CHUNK..MAX_CHUNK chars, so a hit is a readable passage.
pub fn chunk(text: &str) -> Vec<String> {
    let (mut out, mut cur) = (Vec::new(), String::new());
    let flush = |cur: &mut String, out: &mut Vec<String>| {
        if !cur.trim().is_empty() {
            out.push(cur.trim().to_string());
        }
        cur.clear();
    };
    for para in text.split("\n\n").map(str::trim).filter(|p| !p.is_empty()) {
        for piece in split_long(para) {
            if cur.len() + piece.len() > MAX_CHUNK {
                flush(&mut cur, &mut out);
            }
            cur.push_str(&piece);
            cur.push_str("\n\n");
            if cur.len() >= MIN_CHUNK {
                flush(&mut cur, &mut out);
            }
        }
    }
    flush(&mut cur, &mut out);
    out
}

/// A paragraph longer than MAX_CHUNK is cut at the last space before the limit.
fn split_long(p: &str) -> Vec<String> {
    let (mut out, mut rest) = (Vec::new(), p);
    while rest.len() > MAX_CHUNK {
        let mut cut = MAX_CHUNK;
        while !rest.is_char_boundary(cut) {
            cut -= 1;
        }
        let at = rest[..cut].rfind(' ').unwrap_or(cut);
        out.push(rest[..at].to_string());
        rest = rest[at..].trim_start();
    }
    out.push(rest.to_string());
    out
}

pub fn hash(text: &str) -> String {
    // FNV-1a: stable across runs and Rust versions, which DefaultHasher doesn't promise.
    let h = text.bytes().fold(0xcbf29ce484222325u64, |h, b| (h ^ b as u64).wrapping_mul(0x100000001b3));
    format!("{h:016x}")
}

const STOP: [&str; 36] = [
    "the", "and", "for", "are", "but", "not", "you", "all", "can", "had", "her", "was", "one", "our", "out", "what", "when",
    "where", "which", "who", "how", "why", "does", "did", "this", "that", "with", "from", "about", "tell", "explain", "please",
    "give", "have", "has", "into",
];

/// Content words of a question, lowercased. Used both to build the FTS query and to score hits.
pub fn terms(q: &str) -> Vec<String> {
    let mut t: Vec<String> = q
        .to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| w.len() >= 3 && !STOP.contains(w))
        .map(String::from)
        .collect();
    t.sort();
    t.dedup();
    t
}

/// FTS5 query: every term quoted (so punctuation can't break the syntax), OR-ed together.
pub fn fts_query(terms: &[String]) -> String {
    terms.iter().map(|t| format!("\"{t}\"")).collect::<Vec<_>>().join(" OR ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chunks_are_readable_passages() {
        let para = "Deadlock needs four conditions. ".repeat(20); // ~640 chars
        let text = format!("{para}\n\n{para}\n\nshort tail");
        let c = chunk(&text);
        assert!(c.len() >= 2 && c.iter().all(|x| x.len() <= MAX_CHUNK + 4), "{:?}", c.iter().map(|x| x.len()).collect::<Vec<_>>());
        assert!(c.last().unwrap().ends_with("short tail"));
        let huge = "word ".repeat(1000);
        assert!(chunk(&huge).iter().all(|x| x.len() <= MAX_CHUNK + 4));
    }

    #[test]
    fn query_terms() {
        assert_eq!(terms("What is the Banker's algorithm for deadlock?"), vec!["algorithm", "banker", "deadlock"]);
        assert_eq!(fts_query(&terms("page replacement")), "\"page\" OR \"replacement\"");
        assert!(terms("hi").is_empty());
    }

    #[test]
    fn hash_is_stable() {
        assert_eq!(hash("abc"), hash("abc"));
        assert_ne!(hash("abc"), hash("abd"));
    }
}
