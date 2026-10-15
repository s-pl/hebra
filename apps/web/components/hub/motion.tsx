"use client";

import { AnimatePresence, MotionConfig, motion, type Transition } from "motion/react";
import type { ReactNode } from "react";

/**
 * Motion vocabulary for the hub. One spring for everything that moves, short
 * fades for everything that appears, and nothing at all for people who
 * prefer reduced motion (MotionConfig reducedMotion="user").
 */
export const spring: Transition = { type: "spring", duration: 0.45, bounce: 0.12 };
export const quick: Transition = { type: "spring", duration: 0.28, bounce: 0 };

export function MotionRoot({ children }: { children: ReactNode }) {
  return (
    <MotionConfig reducedMotion="user" transition={spring}>
      {children}
    </MotionConfig>
  );
}

/** Content arriving: a short rise with a touch of blur, once. */
export function Reveal({
  children,
  delay = 0,
  className,
}: {
  children: ReactNode;
  delay?: number;
  className?: string;
}) {
  return (
    <motion.div
      className={className}
      initial={{ opacity: 0, y: 10, filter: "blur(4px)" }}
      animate={{ opacity: 1, y: 0, filter: "blur(0px)" }}
      transition={{ ...spring, delay }}
    >
      {children}
    </motion.div>
  );
}

const list = { hidden: {}, shown: { transition: { staggerChildren: 0.035, delayChildren: 0.05 } } };
const item = {
  hidden: { opacity: 0, y: 8 },
  shown: { opacity: 1, y: 0, transition: spring },
};

/** A list whose items cascade in. Items must be `StaggerItem`s. */
export function Stagger({
  children,
  className,
  as = "ul",
}: {
  children: ReactNode;
  className?: string;
  as?: "ul" | "ol" | "div";
}) {
  const Tag = as === "ol" ? motion.ol : as === "div" ? motion.div : motion.ul;
  return (
    <Tag className={className} variants={list} initial="hidden" animate="shown">
      {children}
    </Tag>
  );
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
  const Tag = as === "div" ? motion.div : motion.li;
  return (
    <Tag className={className} variants={item}>
      {children}
    </Tag>
  );
}

/**
 * A live list: items that join slide in, items that leave fade out, and the
 * rest glide to their new place (layout animation). Used for "working now"
 * and reviews, which change in real time.
 */
export function LiveList({
  items,
  className,
}: {
  items: { key: string; node: ReactNode; className?: string }[];
  className?: string;
}) {
  return (
    <ul className={className}>
      <AnimatePresence initial={false} mode="popLayout">
        {items.map((i) => (
          <motion.li
            key={i.key}
            layout
            className={i.className}
            initial={{ opacity: 0, scale: 0.96, y: 8 }}
            animate={{ opacity: 1, scale: 1, y: 0 }}
            exit={{ opacity: 0, scale: 0.96, transition: quick }}
          >
            {i.node}
          </motion.li>
        ))}
      </AnimatePresence>
    </ul>
  );
}

/** Swaps content with a crossfade when `id` changes (status pills, counters). */
export function Swap({ id, children, className }: { id: string; children: ReactNode; className?: string }) {
  return (
    <AnimatePresence mode="popLayout" initial={false}>
      <motion.span
        key={id}
        className={className}
        initial={{ opacity: 0, y: 6, filter: "blur(3px)" }}
        animate={{ opacity: 1, y: 0, filter: "blur(0px)" }}
        exit={{ opacity: 0, y: -6, filter: "blur(3px)", transition: quick }}
      >
        {children}
      </motion.span>
    </AnimatePresence>
  );
}
