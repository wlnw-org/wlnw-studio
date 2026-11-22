import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import UpdateBanner from "./UpdateBanner";
import "./styles.css";
import { initHome } from "./paths";

void initHome();

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <App />
    <UpdateBanner />
  </React.StrictMode>,
);
