import { useEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { Map as MLMap, Marker, NavigationControl, setWorkerUrl } from "maplibre-gl";
import "maplibre-gl/dist/maplibre-gl.css";
// MapLibre 6 runs its tile worker as an ES module; Vite bundles it (with the shared chunk it imports) and gives its URL.
import workerUrl from "maplibre-gl/dist/maplibre-gl-worker.mjs?worker&url";

setWorkerUrl(workerUrl);

const STYLE = "https://tiles.openfreemap.org/styles/positron";

/** Click the map (or drag the pin) to choose where a recorder stood. Needs an internet connection for the map tiles;
 * latitude and longitude can always be typed instead. */
export default function MapPicker({ lat, lon, title, onPick, onClose }: { lat: number | null; lon: number | null; title: string; onPick: (lat: number, lon: number) => void; onClose: () => void }) {
  const box = useRef<HTMLDivElement>(null);
  const [pt, setPt] = useState<[number, number] | null>(lat != null && lon != null ? [lat, lon] : null);
  const [failed, setFailed] = useState(false);

  useEffect(() => {
    if (!box.current) return;
    const map = new MLMap({
      container: box.current, style: STYLE, attributionControl: { compact: true },
      center: pt ? [pt[1], pt[0]] : [40, 10], zoom: pt ? 8 : 1.6,
    });
    map.addControl(new NavigationControl({ showCompass: false }), "top-right");
    const marker = new Marker({ draggable: true, color: "#0e7c66" });
    if (pt) marker.setLngLat([pt[1], pt[0]]).addTo(map);
    const set = (lng: number, la: number) => { marker.setLngLat([lng, la]).addTo(map); setPt([Number(la.toFixed(5)), Number(lng.toFixed(5))]); };
    map.on("click", (e) => set(e.lngLat.lng, e.lngLat.lat));
    marker.on("dragend", () => { const p = marker.getLngLat(); set(p.lng, p.lat); });
    map.on("error", () => setFailed(true));
    return () => map.remove();
    // The map is created once; later picks only move the marker.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  return createPortal(
    <div className="modal" onClick={onClose}>
      <div className="modal-card map-card" onClick={(e) => e.stopPropagation()}>
        <header className="head">
          <div><h3>Where was {title}?</h3><p className="muted small">Click the map or drag the pin. Zoom in for precision.</p></div>
          <button className="link" onClick={onClose}>Close</button>
        </header>
        <div ref={box} className="map-box" />
        {failed && <p className="warn small">The map could not load (no internet connection?). You can still type the latitude and longitude.</p>}
        <div className="row map-foot">
          <span className="mono">{pt ? `${pt[0].toFixed(5)}, ${pt[1].toFixed(5)}` : "No place chosen"}</span>
          <button className="primary" disabled={!pt} onClick={() => pt && onPick(pt[0], pt[1])}>Use this place</button>
        </div>
      </div>
    </div>,
    document.body,
  );
}
