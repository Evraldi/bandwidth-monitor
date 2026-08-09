const STORAGE_KEY = 'bm_dark_mode';

export function applyDarkMode(dark: boolean): void {
  const root = document.documentElement;
  if (dark) {
    root.classList.add('dark');
  } else {
    root.classList.remove('dark');
  }
  try {
    localStorage.setItem(STORAGE_KEY, dark ? '1' : '0');
  } catch {
    // ignore storage errors (e.g. disabled localStorage)
  }
}

export function readDarkModeFromCache(): boolean | null {
  try {
    const value = localStorage.getItem(STORAGE_KEY);
    if (value === null) return null;
    return value === '1';
  } catch {
    return null;
  }
}
