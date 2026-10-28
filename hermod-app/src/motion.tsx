// 页面级动效与控件内置动效分工；系统减少动态效果时即时切换。
import React, { useLayoutEffect, useRef, useState } from "react";
import { flushSync } from "react-dom";

export const reducedMotion = () =>
  matchMedia("(prefers-reduced-motion: reduce)").matches;
const supportsTransitions = () =>
  typeof document.startViewTransition === "function";
let activeTransition: ViewTransition | undefined;

export function transition(update: () => void) {
  if (reducedMotion() || !supportsTransitions()) {
    update();
    return;
  }
  activeTransition?.skipTransition();
  const next = document.startViewTransition(() => flushSync(update));
  activeTransition = next;
  void next.ready.catch(() => {});
  void next.finished
    .finally(() => {
      if (activeTransition === next) activeTransition = undefined;
    })
    .catch(() => {});
}

export function MotionRegion({
  motionKey,
  className,
  children,
}: {
  motionKey: string;
  className: string;
  children: React.ReactNode;
}) {
  const ref = useRef<HTMLDivElement>(null);
  useLayoutEffect(() => {
    if (reducedMotion() || supportsTransitions()) return;
    const animation = ref.current?.animate([{ opacity: 0 }, { opacity: 1 }], {
      duration: 180,
      easing: "cubic-bezier(0.2, 0, 0, 1)",
    });
    return () => animation?.cancel();
  }, [motionKey]);
  return (
    <div ref={ref} className={className}>
      {children}
    </div>
  );
}

export function Presence({
  visible,
  children,
}: {
  visible: boolean;
  children: React.ReactNode;
}) {
  const [present, setPresent] = useState(visible);
  const ref = useRef<HTMLDivElement>(null);
  const previous = useRef(children);
  if (visible) previous.current = children;
  useLayoutEffect(() => {
    if (visible) setPresent(true);
    if (!ref.current || reducedMotion()) {
      setPresent(visible);
      return;
    }
    const animation = ref.current.animate(
      visible
        ? [
            { opacity: 0, transform: "translateY(4px)" },
            { opacity: 1, transform: "none" },
          ]
        : [{ opacity: 1 }, { opacity: 0 }],
      {
        duration: visible ? 180 : 100,
        easing: "cubic-bezier(0.2, 0, 0, 1)",
        fill: "both",
      },
    );
    void animation.finished
      .then(() => {
        if (!visible) setPresent(false);
      })
      .catch(() => {});
    return () => animation.cancel();
  }, [visible]);
  if (!visible && !present) return null;
  return (
    <div ref={ref} inert={!visible} aria-hidden={!visible}>
      {visible ? children : previous.current}
    </div>
  );
}
