import { useEffect, useRef, useState } from "react";
import { colors, PROGRESS_HEIGHT, animation } from "../tokens";

interface ProgressBarProps {
  isLoading: boolean;
}

export function ProgressBar({ isLoading }: ProgressBarProps) {
  const [phase, setPhase] = useState<"idle" | "loading" | "complete" | "fading">("idle");
  const barRef = useRef<HTMLDivElement>(null);
  const prevLoading = useRef(false);

  useEffect(() => {
    if (isLoading && !prevLoading.current) {
      setPhase("loading");
    } else if (!isLoading && prevLoading.current) {
      setPhase("complete");
      const timer = setTimeout(() => setPhase("fading"), animation.progressCompleteDuration);
      return () => clearTimeout(timer);
    }
    prevLoading.current = isLoading;
  }, [isLoading]);

  useEffect(() => {
    if (phase === "fading") {
      const timer = setTimeout(() => setPhase("idle"), animation.progressFadeDuration);
      return () => clearTimeout(timer);
    }
  }, [phase]);

  if (phase === "idle") return null;

  const barStyle: React.CSSProperties = {
    position: "absolute",
    bottom: 0,
    left: 0,
    height: PROGRESS_HEIGHT,
    background: colors.accent,
    borderRadius: 1,
    ...(phase === "loading"
      ? {
          width: "75%",
          animation: `progress-loading ${animation.progressLoadDuration}ms ease-out forwards`,
        }
      : phase === "complete"
        ? {
            width: "100%",
            transition: `width ${animation.progressCompleteDuration}ms ease-out`,
            opacity: 1,
          }
        : {
            width: "100%",
            opacity: 0,
            transition: `opacity ${animation.progressFadeDuration}ms ease-out`,
          }),
  };

  return <div ref={barRef} style={barStyle} />;
}
