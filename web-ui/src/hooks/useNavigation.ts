import { useCallback } from "react";

function apiBase(): string {
  return "";
}

function fireAndForget(path: string, body?: object) {
  const base = apiBase();
  fetch(`${base}${path}`, {
    method: "POST",
    headers: body ? { "Content-Type": "application/json" } : undefined,
    body: body ? JSON.stringify(body) : undefined,
  }).catch(() => {
    // fire-and-forget: state updates arrive via SSE
  });
}

export function useNavigation() {
  const goBack = useCallback(() => fireAndForget("/chrome/back"), []);
  const goForward = useCallback(() => fireAndForget("/chrome/forward"), []);
  const reload = useCallback(() => fireAndForget("/chrome/reload"), []);
  const stop = useCallback(() => fireAndForget("/chrome/stop"), []);
  const navigate = useCallback(
    (url: string) => fireAndForget("/chrome/navigate", { url }),
    [],
  );

  return { goBack, goForward, reload, stop, navigate };
}
