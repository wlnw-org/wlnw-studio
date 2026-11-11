import { useEffect, useState } from "react";
import { openUrl } from "@tauri-apps/plugin-opener";
import { api, type Credit } from "./api";
import { useT } from "./i18n";

let cache: Credit[] | null = null;

/** The model credits, loaded once. */
export function useCredits(): Credit[] {
  const [list, setList] = useState<Credit[]>(cache ?? []);
  useEffect(() => { if (!cache) api.credits().then((c) => { cache = c; setList(c); }).catch(() => {}); }, []);
  return list;
}

/** The credit for a Studio model id (imported BirdNET results credit BirdNET). */
export function creditFor(list: Credit[], model: string): Credit | undefined {
  const id = model.startsWith("birdnet") || model === "import:birdnet-analyzer" || model === "import:birdnet-pi" ? "birdnet-2.4"
    : model.startsWith("perch") ? "perch-2.0" : model.startsWith("batdetect2") ? "batdetect2-uk" : model.startsWith("speciesnet") ? "speciesnet" : model;
  return list.find((c) => c.id === id);
}

const ext = (url: string) => (e: React.MouseEvent) => { e.preventDefault(); openUrl(url).catch(() => {}); };

/** One line under a model: who made it, with links to the paper and the authors' page. */
export function CreditLine({ model }: { model: string }) {
  const t = useT();
  const c = creditFor(useCredits(), model);
  if (!c) return null;
  return (
    <small className="block muted credit-line">
      {t("By {by}.", { by: c.by })}{" "}
      <a href={c.paper_url} onClick={ext(c.paper_url)}>{t("Paper")}</a>{" · "}
      <a href={c.source_url} onClick={ext(c.source_url)}>{t("Source")}</a>
    </small>
  );
}

/** "Credits and licences" link and its window. */
export function CreditsLink() {
  const t = useT();
  const [show, setShow] = useState(false);
  const list = useCredits();
  return (
    <>
      <button className="link small" onClick={() => setShow(true)}>{t("Credits and licences")}</button>
      {show && (
        <div className="modal" role="dialog" aria-label={t("Credits and licences")} onClick={() => setShow(false)}>
          <div className="modal-card credits" onClick={(e) => e.stopPropagation()}>
            <h2>{t("Credits and licences")}</h2>
            <p>{t("Studio's own code is free and open source under the Apache 2.0 licence.")}{" "}
              {t("The part of Studio that reproduces BatDetect2 is under BatDetect2's own licence, CC BY-NC 4.0, so it is for non-commercial use only.")}{" "}
              {t("Each model keeps its own licence, listed below. Exports name the model and its paper, so please cite them too.")}</p>
            {list.map((c) => (
              <div key={c.id} className="credit">
                <h3>{c.name}</h3>
                <p className="muted small">{t("By {by}.", { by: c.by })} {t("Licence")} <a href={c.licence_url} onClick={ext(c.licence_url)}>{c.licence}</a>. {c.note}</p>
                <p className="small">{c.citation}</p>
                <p className="small"><a href={c.paper_url} onClick={ext(c.paper_url)}>{t("Paper")}</a>{" · "}<a href={c.source_url} onClick={ext(c.source_url)}>{t("Source")}</a></p>
              </div>
            ))}
            <p className="muted small">{t("Map tiles in the place picker come from OpenFreeMap, with data from OpenStreetMap contributors.")}</p>
            <div className="row"><button className="primary" onClick={() => setShow(false)}>{t("Close")}</button></div>
          </div>
        </div>
      )}
    </>
  );
}
