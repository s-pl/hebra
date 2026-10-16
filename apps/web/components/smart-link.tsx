import Link from "next/link";
import type { ComponentProps } from "react";

/**
 * A link that goes through Next's router for paths on this site (so they
 * get the base path and client navigation) and stays a plain anchor for
 * other sites and mail links.
 */
export function SmartLink({ href, ...rest }: ComponentProps<"a"> & { href: string }) {
  if (href.startsWith("/")) return <Link href={href} {...rest} />;
  return <a href={href} {...rest} />;
}
