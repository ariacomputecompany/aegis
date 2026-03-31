import {
  TOOLBAR_HEIGHT,
  NAV_LEFT_INSET,
  ADDR_GAP,
  ADDR_RIGHT_INSET,
  TAB_LEFT_INSET,
} from "../tokens";
import { TabStrip } from "./TabStrip";
import { NavigationButtons } from "./NavigationButtons";
import { AddressBar } from "./AddressBar";
import { ProgressBar } from "./ProgressBar";
import { Separator } from "./Separator";

interface ToolbarProps {
  title: string;
  url: string;
  canGoBack: boolean;
  canGoForward: boolean;
  isLoading: boolean;
  onBack: () => void;
  onForward: () => void;
  onReload: () => void;
  onStop: () => void;
  onNavigate: (url: string) => void;
}

export function Toolbar({
  title,
  url,
  canGoBack,
  canGoForward,
  isLoading,
  onBack,
  onForward,
  onReload,
  onStop,
  onNavigate,
}: ToolbarProps) {
  return (
    <div
      style={{
        height: TOOLBAR_HEIGHT,
        flexShrink: 0,
        display: "flex",
        flexDirection: "column",
        position: "relative",
        background: "rgba(249, 249, 249, 0.85)",
        backdropFilter: "blur(20px) saturate(1.8)",
        WebkitBackdropFilter: "blur(20px) saturate(1.8)",
      }}
    >
      {/* Upper band: tab strip */}
      <div
        style={{
          display: "flex",
          alignItems: "center",
          paddingLeft: TAB_LEFT_INSET,
          paddingRight: ADDR_RIGHT_INSET,
          height: TOOLBAR_HEIGHT / 2,
        }}
      >
        <TabStrip title={title} />
      </div>

      {/* Lower band: navigation + address bar */}
      <div
        style={{
          display: "flex",
          alignItems: "center",
          paddingLeft: NAV_LEFT_INSET,
          paddingRight: ADDR_RIGHT_INSET,
          height: TOOLBAR_HEIGHT / 2,
          gap: ADDR_GAP,
        }}
      >
        <NavigationButtons
          canGoBack={canGoBack}
          canGoForward={canGoForward}
          isLoading={isLoading}
          onBack={onBack}
          onForward={onForward}
          onReload={onReload}
          onStop={onStop}
        />
        <AddressBar url={url} onNavigate={onNavigate} />
      </div>

      {/* Progress bar at the very bottom */}
      <ProgressBar isLoading={isLoading} />

      {/* Separator */}
      <Separator />
    </div>
  );
}
