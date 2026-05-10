import type { MouseEvent } from "react";
import {
  TAB_HEIGHT,
  TAB_RADIUS,
  TAB_H_PADDING,
  TAB_MIN_WIDTH,
  TAB_MAX_WIDTH,
  NEW_TAB_BUTTON_SIZE,
  colors,
} from "../tokens";
import type { BrowserTabState } from "../hooks/useChromeState";
import { Plus, XMark } from "../icons";

interface TabStripProps {
  tabs: BrowserTabState[];
  activeTabId: number;
  fallbackTitle: string;
  onCreateTab: (url?: string) => void;
  onActivateTab: (tabId: number) => void;
  onCloseTab: (tabId: number) => void;
}

export function TabStrip({
  tabs,
  activeTabId,
  fallbackTitle,
  onCreateTab,
  onActivateTab,
  onCloseTab,
}: TabStripProps) {
  const safeTabs =
    tabs.length > 0
      ? tabs
      : [
          {
            id: 1,
            title: fallbackTitle || "New Tab",
            url: "",
            canGoBack: false,
            canGoForward: false,
            isLoading: false,
          },
        ];
  const tabWidth = Math.max(
    TAB_MIN_WIDTH,
    Math.min(TAB_MAX_WIDTH, Math.floor(680 / Math.max(safeTabs.length, 1))),
  );

  const handleClose = (event: MouseEvent<HTMLButtonElement>, tabId: number) => {
    event.stopPropagation();
    onCloseTab(tabId);
  };

  return (
    <div style={{ display: "flex", alignItems: "center", gap: 6, minWidth: 0, width: "100%" }}>
      <div style={{ display: "flex", alignItems: "center", gap: 4, minWidth: 0, flex: 1 }}>
        {safeTabs.map((tab) => {
          const active = tab.id === activeTabId;
          return (
            <div
              key={tab.id}
              role="button"
              tabIndex={0}
              onClick={() => onActivateTab(tab.id)}
              onKeyDown={(event) => {
                if (event.key === "Enter" || event.key === " ") {
                  event.preventDefault();
                  onActivateTab(tab.id);
                }
              }}
              style={{
                height: TAB_HEIGHT,
                width: tabWidth,
                minWidth: TAB_MIN_WIDTH,
                maxWidth: TAB_MAX_WIDTH,
                borderRadius: TAB_RADIUS,
                background: active ? colors.tabBg : "rgba(255, 255, 255, 0.42)",
                border: `0.5px solid ${active ? colors.activeTabBorder : "rgba(0, 0, 0, 0.05)"}`,
                boxShadow: active ? colors.tabShadow : "none",
                display: "flex",
                alignItems: "center",
                gap: 8,
                paddingLeft: TAB_H_PADDING,
                paddingRight: 8,
                overflow: "hidden",
                color: colors.primaryText,
                flexShrink: 0,
                cursor: active ? "default" : "pointer",
                opacity: active ? 1 : 0.82,
              }}
            >
              <span
                style={{
                  fontSize: 12.5,
                  fontWeight: active ? 600 : 500,
                  color: colors.primaryText,
                  whiteSpace: "nowrap",
                  overflow: "hidden",
                  textOverflow: "ellipsis",
                  lineHeight: `${TAB_HEIGHT}px`,
                  flex: 1,
                  textAlign: "left",
                }}
              >
                {tab.title || tab.url || "New Tab"}
              </span>
              <span
                onClick={(event) => event.stopPropagation()}
                style={{ display: "flex", alignItems: "center", flexShrink: 0 }}
              >
                <button
                  aria-label={`Close ${tab.title || "tab"}`}
                  onClick={(event) => handleClose(event, tab.id)}
                  style={{
                    width: 18,
                    height: 18,
                    borderRadius: 9,
                    display: "flex",
                    alignItems: "center",
                    justifyContent: "center",
                    color: colors.secondaryText,
                    cursor: "pointer",
                    flexShrink: 0,
                    background: active ? "rgba(0, 0, 0, 0.05)" : "transparent",
                  }}
                >
                  <XMark size={10} />
                </button>
              </span>
            </div>
          );
        })}
      </div>

      <button
        aria-label="New tab"
        onClick={() => onCreateTab()}
        style={{
          width: NEW_TAB_BUTTON_SIZE,
          height: NEW_TAB_BUTTON_SIZE,
          borderRadius: NEW_TAB_BUTTON_SIZE / 2,
          display: "flex",
          alignItems: "center",
          justifyContent: "center",
          cursor: "pointer",
          color: colors.navIconDefault,
          flexShrink: 0,
          background: "rgba(255, 255, 255, 0.4)",
        }}
      >
        <Plus />
      </button>
    </div>
  );
}
