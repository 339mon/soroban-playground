"use client";

import React from "react";

export default function ThemeSwitcher() {
  const [theme, setTheme] = React.useState<string | null>(null);

  React.useEffect(() => {
    try {
      const stored = typeof window !== "undefined" ? window.localStorage.getItem("sp:theme") : null;
      if (stored === "light" || stored === "dark") {
        setTheme(stored);
        if (typeof document !== "undefined") document.documentElement.setAttribute("data-theme", stored);
        return;
      }
      // respect prefers-color-scheme when available
      if (typeof window !== "undefined" && window.matchMedia) {
        const prefersDark = window.matchMedia("(prefers-color-scheme: dark)").matches;
        const initial = prefersDark ? "dark" : "light";
        setTheme(initial);
        if (typeof document !== "undefined") document.documentElement.setAttribute("data-theme", initial);
        return;
      }
      setTheme("dark");
      if (typeof document !== "undefined") document.documentElement.setAttribute("data-theme", "dark");
    } catch (e) {
      console.error("ThemeSwitcher:init error", e);
      setTheme("dark");
    }
  }, []);

  const toggle = () => {
    try {
      const next = theme === "dark" ? "light" : "dark";
      setTheme(next);
      if (typeof document !== "undefined") document.documentElement.setAttribute("data-theme", next);
      if (typeof window !== "undefined" && window.localStorage) {
        window.localStorage.setItem("sp:theme", next);
      }
    } catch (e) {
      console.error("ThemeSwitcher:toggle error", e);
    }
  };

  return (
    <div>
      <button
        aria-label="Toggle theme"
        onClick={toggle}
        className="px-2 py-1 rounded bg-white/5 hover:bg-white/10 text-sm"
      >
        {theme === "dark" ? "Dark" : theme === "light" ? "Light" : "Theme"}
      </button>
    </div>
  );
}
