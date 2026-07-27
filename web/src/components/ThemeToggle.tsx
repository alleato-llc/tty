import { useEffect, useState } from "preact/hooks";

type Mode = "light" | "dark";
/** The four themes the page can wear. `dark`/`light` are the standard pair
 *  (Dracula / Solarized Light); `phosphor` and `github` are only reachable via
 *  the theme-grid easter egg on slide 7. */
type Theme = Mode | "phosphor" | "github";

/** Which of the two captures a theme shows, and which way the toggle should
 *  flip out of it. */
const IS_DARK: Record<Theme, boolean> = {
  dark: true,
  phosphor: true,
  light: false,
  github: false,
};

/** The visitor's standing preference, ignoring anything the easter egg set. */
function standardTheme(): Mode {
  const saved = localStorage.getItem("tty-theme");
  if (saved === "light" || saved === "dark") return saved;
  return matchMedia("(prefers-color-scheme: dark)").matches ? "dark" : "light";
}

/** Light/dark toggle. The inline script in Layout.astro resolves the system
 *  preference before first paint; this overrides it and remembers the choice.
 *  Dark is tty's Dracula; light is Solarized Light.
 *
 *  It doubles as the escape hatch from the theme-grid easter egg: while a picked
 *  theme is active (`data-easter`), the first press RESTORES the visitor's own
 *  preference rather than flipping, so the standard control always undoes the
 *  egg in one click. */
export default function ThemeToggle() {
  const [theme, setTheme] = useState<Theme>("dark");

  useEffect(() => {
    const read = () =>
      setTheme((document.documentElement.dataset.theme as Theme) ?? "dark");
    read();
    // The egg and the system-preference listener both change data-theme
    // without going through this component.
    const mo = new MutationObserver(read);
    mo.observe(document.documentElement, { attributeFilter: ["data-theme"] });
    return () => mo.disconnect();
  }, []);

  const flip = () => {
    const root = document.documentElement;
    let next: Mode;
    if (root.dataset.easter) {
      // Revert out of the easter egg to whatever the visitor actually prefers.
      // Nothing is written: the egg never persisted, so there is nothing to undo
      // beyond dropping the marker.
      delete root.dataset.easter;
      const std = standardTheme();
      // If the picked theme happens to equal the standing preference (someone
      // clicked the Dracula card while already on dark), reverting would be a
      // no-op and the button would feel broken — flip instead.
      next = std === theme ? (IS_DARK[theme] ? "light" : "dark") : std;
    } else {
      next = theme === "light" ? "dark" : "light";
      localStorage.setItem("tty-theme", next);
    }
    root.dataset.theme = next;
    setTheme(next);
  };

  const dark = IS_DARK[theme] ?? true;
  const target = dark ? "light" : "dark";
  return (
    <button
      class="theme-toggle"
      onClick={flip}
      aria-label={`Switch to ${target} theme`}
      title={`Switch to ${target} theme`}
    >
      {dark ? "◑" : "◐"}
    </button>
  );
}
