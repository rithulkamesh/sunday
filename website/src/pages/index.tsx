import Layout from "@theme/Layout";
import Particles from "../components/Particles";
import s from "./index.module.css";

export default function Home() {
  return (
    <Layout noFooter title="Can any model go AGI?" description="Sunday is an open experiment: a local-first agent harness around a model you can swap.">
      <main className={s.stage}>
        <Particles className={s.canvas} />
        <div className={s.copy}>
          <h1 className={s.name} aria-label="Sunday">
            {[..."SUNDAY"].map((l, i) => (
              <span key={i} aria-hidden="true">
                {l}
                <i>.</i>
              </span>
            ))}
          </h1>
          <p className={s.question}>Can any model go AGI, given the right harness?</p>
        </div>
      </main>
    </Layout>
  );
}
