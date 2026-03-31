import { useRef, useEffect, useState } from "react";
import {
  TAB_HEIGHT,
  TAB_RADIUS,
  TAB_H_PADDING,
  TAB_MIN_WIDTH,
  TAB_MAX_WIDTH,
  TAB_INITIAL_WIDTH,
  NEW_TAB_BUTTON_SIZE,
  colors,
} from "../tokens";
import { Plus } from "../icons";

interface TabStripProps {
  title: string;
}

export function TabStrip({ title }: TabStripProps) {
  const [tabWidth, setTabWidth] = useState(TAB_INITIAL_WIDTH);
  const measureRef = useRef<HTMLSpanElement>(null);

  useEffect(() => {
    if (measureRef.current) {
      const textWidth = measureRef.current.offsetWidth + TAB_H_PADDING * 2;
      setTabWidth(Math.max(TAB_MIN_WIDTH, Math.min(TAB_MAX_WIDTH, textWidth)));
    }
  }, [title]);

  return (
    <div style={{ display: "flex", alignItems: "center", gap: 4 }}>
      {/* Tab background */}
      <div
        style={{
          height: TAB_HEIGHT,
          width: tabWidth,
          borderRadius: TAB_RADIUS,
          background: colors.tabBg,
          border: `0.5px solid ${colors.activeTabBorder}`,
          boxShadow: colors.tabShadow,
          display: "flex",
          alignItems: "center",
          paddingLeft: TAB_H_PADDING,
          paddingRight: TAB_H_PADDING,
          overflow: "hidden",
        }}
      >
        <span
          style={{
            fontSize: 12.5,
            fontWeight: 500,
            color: colors.primaryText,
            whiteSpace: "nowrap",
            overflow: "hidden",
            textOverflow: "ellipsis",
            lineHeight: `${TAB_HEIGHT}px`,
          }}
        >
          {title || "New Tab"}
        </span>
      </div>

      {/* Hidden text measurer */}
      <span
        ref={measureRef}
        aria-hidden
        style={{
          position: "absolute",
          visibility: "hidden",
          fontSize: 12.5,
          fontWeight: 500,
          whiteSpace: "nowrap",
        }}
      >
        {title || "New Tab"}
      </span>

      {/* New tab button (disabled) */}
      <button
        disabled
        style={{
          width: NEW_TAB_BUTTON_SIZE,
          height: NEW_TAB_BUTTON_SIZE,
          borderRadius: NEW_TAB_BUTTON_SIZE / 2,
          display: "flex",
          alignItems: "center",
          justifyContent: "center",
          opacity: 0.5,
          cursor: "default",
          color: colors.navIconDefault,
          flexShrink: 0,
        }}
      >
        <Plus />
      </button>
    </div>
  );
}
