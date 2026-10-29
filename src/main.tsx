import React from "react";
import { createRoot } from "react-dom/client";
import App from "./App";
import "./styles/tokens.css";
import "./styles/surface.css";
import "./styles/app.css";

const el = document.getElementById("root");
if (!el) throw new Error("找不到 #root —— index.html 被改坏了");

createRoot(el).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
