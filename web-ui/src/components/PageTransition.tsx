import { useEffect, useRef, useState } from "react";
import { colors, animation } from "../tokens";

interface PageTransitionProps {
  isLoading: boolean;
}

export function PageTransition({ isLoading }: PageTransitionProps) {
  const [visible, setVisible] = useState(false);
  const [opacity, setOpacity] = useState(0);
  const showTimerRef = useRef<ReturnType<typeof setTimeout>>(undefined);
  const visibleSinceRef = useRef<number>(0);

  useEffect(() => {
    if (isLoading) {
      showTimerRef.current = setTimeout(() => {
        visibleSinceRef.current = Date.now();
        setVisible(true);
        requestAnimationFrame(() => setOpacity(animation.pageTransitionMaxAlpha));
      }, animation.pageTransitionDelay);
    } else {
      if (showTimerRef.current) {
        clearTimeout(showTimerRef.current);
        showTimerRef.current = undefined;
      }
      if (visible) {
        const elapsed = Date.now() - visibleSinceRef.current;
        const remaining = Math.max(0, animation.pageTransitionMinVisible - elapsed);
        setTimeout(() => {
          setOpacity(0);
          setTimeout(() => setVisible(false), animation.pageTransitionFadeOut);
        }, remaining);
      }
    }
    return () => {
      if (showTimerRef.current) {
        clearTimeout(showTimerRef.current);
      }
    };
  }, [isLoading, visible]);

  if (!visible) return null;

  return (
    <div
      style={{
        position: "absolute",
        inset: 0,
        background: colors.pageTransition,
        opacity,
        transition: `opacity ${isLoading ? animation.pageTransitionFadeIn : animation.pageTransitionFadeOut}ms ${isLoading ? "ease-out" : "ease-in-out"}`,
        pointerEvents: "none",
      }}
    />
  );
}
