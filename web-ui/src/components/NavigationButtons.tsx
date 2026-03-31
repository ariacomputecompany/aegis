import { useState, type ReactNode } from "react";
import {
  NAV_BUTTON_SIZE,
  NAV_BUTTON_RADIUS,
  NAV_BUTTON_GAP,
  colors,
} from "../tokens";
import { ChevronLeft, ChevronRight, ArrowClockwise, XMark } from "../icons";

interface NavButtonProps {
  icon: ReactNode;
  disabled?: boolean;
  onClick: () => void;
}

function NavButton({ icon, disabled, onClick }: NavButtonProps) {
  const [hovered, setHovered] = useState(false);
  const [pressed, setPressed] = useState(false);

  const bg = disabled
    ? "transparent"
    : pressed
      ? colors.btnPressedBg
      : hovered
        ? colors.btnHoverBg
        : "transparent";

  const iconColor = disabled
    ? colors.navIconDefault
    : hovered
      ? colors.navIconActive
      : colors.navIconDefault;

  return (
    <button
      onClick={onClick}
      disabled={disabled}
      onMouseEnter={() => setHovered(true)}
      onMouseLeave={() => {
        setHovered(false);
        setPressed(false);
      }}
      onMouseDown={() => setPressed(true)}
      onMouseUp={() => setPressed(false)}
      style={{
        width: NAV_BUTTON_SIZE,
        height: NAV_BUTTON_SIZE,
        borderRadius: NAV_BUTTON_RADIUS,
        display: "flex",
        alignItems: "center",
        justifyContent: "center",
        background: bg,
        opacity: disabled ? 0.3 : 1,
        cursor: disabled ? "default" : "pointer",
        transition: "background 100ms",
        color: iconColor,
        flexShrink: 0,
      }}
    >
      {icon}
    </button>
  );
}

interface NavigationButtonsProps {
  canGoBack: boolean;
  canGoForward: boolean;
  isLoading: boolean;
  onBack: () => void;
  onForward: () => void;
  onReload: () => void;
  onStop: () => void;
}

export function NavigationButtons({
  canGoBack,
  canGoForward,
  isLoading,
  onBack,
  onForward,
  onReload,
  onStop,
}: NavigationButtonsProps) {
  return (
    <div
      style={{
        display: "flex",
        alignItems: "center",
        gap: NAV_BUTTON_GAP,
        flexShrink: 0,
      }}
    >
      <NavButton
        icon={<ChevronLeft />}
        disabled={!canGoBack}
        onClick={onBack}
      />
      <NavButton
        icon={<ChevronRight />}
        disabled={!canGoForward}
        onClick={onForward}
      />
      <NavButton
        icon={isLoading ? <XMark /> : <ArrowClockwise />}
        onClick={isLoading ? onStop : onReload}
      />
    </div>
  );
}
