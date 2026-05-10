import { useEffect, useState } from "react";

export interface BrowserTabState {
  id: number;
  title: string;
  url: string;
  canGoBack: boolean;
  canGoForward: boolean;
  isLoading: boolean;
}

export interface BrowserUiState {
  activeTabId: number;
  tabs: BrowserTabState[];
}

const DEFAULT_TAB: BrowserTabState = {
  id: 1,
  title: "Aegis",
  url: "",
  canGoBack: false,
  canGoForward: false,
  isLoading: false,
};

const DEFAULT_STATE: BrowserUiState = {
  activeTabId: 1,
  tabs: [DEFAULT_TAB],
};

function apiBase(): string {
  return "";
}

export function useChromeState(): BrowserUiState {
  const [state, setState] = useState<BrowserUiState>(DEFAULT_STATE);

  useEffect(() => {
    const base = apiBase();
    const source = new EventSource(`${base}/ui/chrome/tabs`);

    source.onmessage = (event) => {
      try {
        const data = JSON.parse(event.data);
        setState({
          activeTabId: data.active_tab_id ?? 1,
          tabs: Array.isArray(data.tabs)
            ? data.tabs.map((tab: Record<string, unknown>) => ({
                id: Number(tab.id ?? 0),
                title: String(tab.title ?? ""),
                url: String(tab.url ?? ""),
                canGoBack: Boolean(tab.can_go_back ?? false),
                canGoForward: Boolean(tab.can_go_forward ?? false),
                isLoading: Boolean(tab.is_loading ?? false),
              }))
            : DEFAULT_STATE.tabs,
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
