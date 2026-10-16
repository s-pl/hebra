"use client";

import { MotionConfig, type Transition } from "motion/react";
import type { ReactNode } from "react";

/**
 * Motion in the hub is kept to feedback (presses, drawers, popovers). Pages
 * and lists render as they are: an app used all day should not make people
 * wait for content to fade in. Nothing moves for people who prefer reduced
 * motion (MotionConfig reducedMotion="user").
 */
export const spring: Transition = { type: "spring", duration: 0.4, bounce: 0 };
export const quick: Transition = { type: "spring", duration: 0.25, bounce: 0 };

export function MotionRoot({ children }: { children: ReactNode }) {
  return (
    <MotionConfig reducedMotion="user" transition={spring}>
      {children}
    </MotionConfig>
  );
}

/** A block of content (kept as a wrapper so pages can group sections). */
export function Reveal({ children, className }: { children: ReactNode; delay?: number; className?: string }) {
  return <div className={className}>{children}</div>;
}

/** A list (`ul`, `ol` or `div`) of `StaggerItem`s. */
export function Stagger({
  children,
  className,
  as = "ul",
}: {
  children: ReactNode;
  className?: string;
  as?: "ul" | "ol" | "div";
}) {
  const Tag = as;
  return <Tag className={className}>{children}</Tag>;
}

export function StaggerItem({
  children,
  className,
  as = "li",
}: {
  children: ReactNode;
  className?: string;
  as?: "li" | "div";
}) {
  const Tag = as;
  return <Tag className={className}>{children}</Tag>;
}
