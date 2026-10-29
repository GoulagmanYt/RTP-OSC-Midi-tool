import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import { LanguageProvider } from "./providers/LanguageProvider";
import "./styles.css";
import { initializeStartupWindow } from "./window/startup-window";
import { applyMotionPreference, readMotionPreference } from "./motion-preference";

applyMotionPreference(readMotionPreference());
void initializeStartupWindow();

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <LanguageProvider>
      <App />
    </LanguageProvider>
  </React.StrictMode>
);
