import React from "react";
import ReactDOM from "react-dom/client";
import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";
import App from "./App";
import TraySettings from "./TraySettings";
import "./styles/globals.css";

const windowLabel = getCurrentWebviewWindow().label;

ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    {windowLabel === "settings" ? <TraySettings /> : <App />}
  </React.StrictMode>
);
