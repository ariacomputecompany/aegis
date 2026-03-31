import { useEffect, useState } from "react";

export interface ChromeState {
  title: string;
  url: string;
  canGoBack: boolean;
  canGoForward: boolean;
  isLoading: boolean;
}

const DEFAULT_STATE: ChromeState = {
  title: "Aegis",
  url: "",
  canGoBack: false,
  canGoForward: false,
  isLoading: false,
};

function apiBase(): string {
  if (import.meta.env.DEV) {
    return "";
  }
  return "";
}

export function useChromeState(): ChromeState {
  const [state, setState] = useState<ChromeState>(DEFAULT_STATE);

  useEffect(() => {
    const base = apiBase();
    const source = new EventSource(`${base}/chrome/state`);

    source.onmessage = (event) => {
      try {
        const data = JSON.parse(event.data);
        setState({
          title: data.title ?? "",
          url: data.url ?? "",
          canGoBack: data.can_go_back ?? false,
          canGoForward: data.can_go_forward ?? false,
          isLoading: data.is_loading ?? false,
        });
      } catch {
        // ignore malformed messages
      }
    };

    source.onerror = () => {
      // EventSource auto-reconnects; no action needed
    };

    return () => source.close();
  }, []);

  return state;
}
