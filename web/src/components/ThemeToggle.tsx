import { useEffect, useState } from "preact/hooks";

type Mode = "light" | "dark";

/** Light/dark toggle. The inline script in Layout.astro resolves the system
 *  preference before first paint; this overrides it and remembers the choice.
 *  Dark is tty's Dracula; light is GitHub Light. */
export default function ThemeToggle() {
  const [mode, setMode] = useState<Mode>("dark");

  useEffect(() => {
    setMode((document.documentElement.dataset.theme as Mode) ?? "dark");
  }, []);

  const flip = () => {
    const next: Mode = mode === "light" ? "dark" : "light";
    document.documentElement.dataset.theme = next;
    localStorage.setItem("tty-theme", next);
    setMode(next);
  };

  const target = mode === "light" ? "dark" : "light";
  return (
    <button
      class="theme-toggle"
      onClick={flip}
      aria-label={`Switch to ${target} theme`}
      title={`Switch to ${target} theme`}
    >
      {mode === "light" ? "◐" : "◑"}
    </button>
  );
}
