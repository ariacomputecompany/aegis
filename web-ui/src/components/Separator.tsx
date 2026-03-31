import { colors, SEPARATOR_HEIGHT } from "../tokens";

export function Separator() {
  return (
    <div
      style={{
        height: SEPARATOR_HEIGHT,
        width: "100%",
        background: colors.separator,
        flexShrink: 0,
      }}
    />
  );
}
