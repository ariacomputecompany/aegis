import { useState, useRef, useCallback, useEffect } from "react";
import {
  ADDR_HEIGHT,
  ADDR_RADIUS,
  ADDR_H_PADDING,
  LOCK_ICON_SIZE,
  colors,
} from "../tokens";
import { LockFill } from "../icons";

interface AddressBarProps {
  url: string;
  onNavigate: (url: string) => void;
}

export function AddressBar({ url, onNavigate }: AddressBarProps) {
  const [focused, setFocused] = useState(false);
  const [hovered, setHovered] = useState(false);
  const [editValue, setEditValue] = useState("");
  const inputRef = useRef<HTMLInputElement>(null);

  const isHttps = url.startsWith("https://");
  const showLock = isHttps && !focused && url.length > 0;

  useEffect(() => {
    if (!focused) {
      setEditValue(url);
    }
  }, [url, focused]);

  const handleFocus = useCallback(() => {
    setFocused(true);
    setEditValue(url);
    requestAnimationFrame(() => inputRef.current?.select());
  }, [url]);

  const handleBlur = useCallback(() => {
    setFocused(false);
  }, []);

  const handleSubmit = useCallback(
    (e: React.FormEvent<HTMLFormElement>) => {
      e.preventDefault();
      let value = editValue.trim();
      if (value.length === 0) return;
      if (!value.includes("://")) {
        value = `https://${value}`;
      }
      onNavigate(value);
      inputRef.current?.blur();
    },
    [editValue, onNavigate],
  );

  const bg = focused
    ? colors.addrFocusBg
    : hovered
      ? colors.addrHoverBg
      : colors.addrBg;
  const borderColor = focused ? colors.addrFocusBorder : colors.addrBorder;
  const borderWidth = focused ? 1.5 : 1;

  return (
    <form
      onSubmit={handleSubmit}
      onMouseEnter={() => setHovered(true)}
      onMouseLeave={() => setHovered(false)}
      style={{
        height: ADDR_HEIGHT,
        borderRadius: ADDR_RADIUS,
        background: bg,
        border: `${borderWidth}px solid ${borderColor}`,
        display: "flex",
        alignItems: "center",
        paddingLeft: ADDR_H_PADDING,
        paddingRight: ADDR_H_PADDING,
        gap: 6,
        flex: 1,
        minWidth: 0,
        transition: "background 150ms, border-color 150ms",
      }}
    >
      {showLock && (
        <LockFill size={LOCK_ICON_SIZE} color={colors.lockIcon} style={{ flexShrink: 0 }} />
      )}
      <input
        ref={inputRef}
        type="text"
        value={focused ? editValue : url}
        onChange={(e) => setEditValue(e.target.value)}
        onFocus={handleFocus}
        onBlur={handleBlur}
        placeholder="Search or enter address"
        style={{
          flex: 1,
          minWidth: 0,
          fontSize: 13,
          fontWeight: 400,
          color: colors.primaryText,
          background: "transparent",
          lineHeight: `${ADDR_HEIGHT - 2}px`,
        }}
      />
    </form>
  );
}
