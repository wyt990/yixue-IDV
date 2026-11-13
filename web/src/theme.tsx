import {
  createContext,
  useContext,
  useEffect,
  useMemo,
  useState,
  type ReactNode,
} from "react";

export type ThemeId = "ink-lab" | "pine-rack" | "slate-ops" | "night-duty";

const THEME_KEY = "idv_theme";
const THEME_IDS: ThemeId[] = ["ink-lab", "pine-rack", "slate-ops", "night-duty"];

type ThemeContextValue = {
  theme: ThemeId;
  setTheme: (theme: ThemeId) => void;
};

const ThemeContext = createContext<ThemeContextValue | null>(null);

function parseTheme(raw: string | null): ThemeId {
  if (raw && (THEME_IDS as string[]).includes(raw)) {
    return raw as ThemeId;
  }
  return "ink-lab";
}

export function ThemeProvider({ children }: { children: ReactNode }) {
  const [theme, setThemeState] = useState<ThemeId>(() =>
    parseTheme(localStorage.getItem(THEME_KEY)),
  );

  useEffect(() => {
    document.documentElement.setAttribute("data-theme", theme);
    localStorage.setItem(THEME_KEY, theme);
  }, [theme]);

  const value = useMemo(
    () => ({
      theme,
      setTheme: setThemeState,
    }),
    [theme],
  );

  return (
    <ThemeContext.Provider value={value}>{children}</ThemeContext.Provider>
  );
}

export function useTheme() {
  const ctx = useContext(ThemeContext);
  if (!ctx) throw new Error("ThemeProvider missing");
  return ctx;
}
