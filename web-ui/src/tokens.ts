// Layout dimensions — extracted from native/aegis_browser_host_mac.inc
export const TOOLBAR_HEIGHT = 82;
export const TAB_HEIGHT = 28;
export const TAB_RADIUS = 8;
export const TAB_LEFT_INSET = 76;
export const TAB_H_PADDING = 12;
export const TAB_INITIAL_WIDTH = 200;
export const TAB_MIN_WIDTH = 80;
export const TAB_MAX_WIDTH = 240;
export const TAB_Y_OFFSET = 34; // from bottom of toolbar
export const NAV_BUTTON_SIZE = 28;
export const NAV_BUTTON_RADIUS = 6;
export const NAV_BUTTON_GAP = 4;
export const NAV_LEFT_INSET = 14;
export const NAV_Y_CENTER = 22; // from bottom of toolbar
export const NEW_TAB_BUTTON_SIZE = 22;
export const ADDR_HEIGHT = 32;
export const ADDR_RADIUS = 8;
export const ADDR_H_PADDING = 10;
export const ADDR_GAP = 10;
export const ADDR_RIGHT_INSET = 14;
export const LOCK_ICON_SIZE = 14;
export const PROGRESS_HEIGHT = 2;
export const SEPARATOR_HEIGHT = 1;

// Colors — from Cocoa SRGB values
export const colors = {
  windowBg: "#f9f9f9",
  tabBg: "#ffffff",
  activeTabBorder: "rgba(0, 0, 0, 0.08)",
  tabShadow: "0 1px 3px rgba(0, 0, 0, 0.08)",
  primaryText: "#1a1a1a",
  secondaryText: "rgba(0, 0, 0, 0.50)",
  placeholderText: "rgba(0, 0, 0, 0.32)",
  addrBg: "rgba(0, 0, 0, 0.055)",
  addrHoverBg: "rgba(0, 0, 0, 0.075)",
  addrFocusBg: "#ffffff",
  addrBorder: "rgba(0, 0, 0, 0.09)",
  addrFocusBorder: "rgba(59, 130, 247, 0.50)",
  btnHoverBg: "rgba(0, 0, 0, 0.06)",
  btnPressedBg: "rgba(0, 0, 0, 0.10)",
  separator: "rgba(0, 0, 0, 0.12)",
  accent: "#3B82F7",
  pageTransition: "#f8f9fb",
  navIconDefault: "#404040",
  navIconActive: "#1f1f1f",
  lockIcon: "rgba(0, 0, 0, 0.36)",
} as const;

// Animation timings — from Cocoa source
export const animation = {
  progressLoadDuration: 8000,
  progressCompleteDuration: 200,
  progressFadeDuration: 300,
  pageTransitionDelay: 50,
  pageTransitionFadeIn: 140,
  pageTransitionMinVisible: 120,
  pageTransitionFadeOut: 180,
  pageTransitionMaxAlpha: 0.16,
} as const;
