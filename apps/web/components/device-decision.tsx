"use client";

import { useRouter } from "next/navigation";
import { useState, useTransition } from "react";
import { type ActionResult, decideDevice } from "@/app/hub/actions";

/** Enter or confirm a device code, then approve or deny it. */
export function DeviceDecision({
  code,
  device,
  problem,
}: {
  code: string;
  device: { client_name: string; status: string } | null;
  problem: string | null;
}) {
  const router = useRouter();
  const [value, setValue] = useState(code);
  const [pending, start] = useTransition();
  const [result, setResult] = useState<ActionResult | null>(null);

  if (result?.ok) {
    return (
      <p role="status" className="text-[14.5px]">
        {result.message}
      </p>
    );
  }

  if (device?.status !== "pending") {
    return (
      <form
        className="grid gap-3"
        onSubmit={(e) => {
          e.preventDefault();
          router.push(`/login/device?code=${encodeURIComponent(value.trim().toUpperCase())}`);
        }}
      >
        <label htmlFor="code" className="font-medium text-[13px]">
          Code from your terminal
        </label>
        <input
          id="code"
          value={value}
          onChange={(e) => setValue(e.target.value)}
          placeholder="ABCD-EFGH"
          autoComplete="one-time-code"
          autoCapitalize="characters"
          spellCheck={false}
          className="h-12 rounded-lg border border-line bg-surface px-3 text-center font-mono text-[20px] uppercase tracking-[0.2em] outline-none focus:border-ink"
        />
        {problem || device ? (
          <p role="alert" className="text-[13px] text-accent">
            {problem ?? `This request is already ${device?.status}.`}
          </p>
        ) : null}
        <button type="submit" className="pressable h-10 rounded-lg bg-ink font-medium text-[14px] text-bg">
          Continue
        </button>
      </form>
    );
  }

  const decide = (approve: boolean) =>
    start(async () => {
      setResult(await decideDevice(code, approve));
    });

  return (
    <div className="grid gap-5">
      <div className="text-center">
        <p className="font-mono font-semibold text-[28px] tracking-[0.2em]">{code}</p>
        <p className="mt-1 text-[13px] text-ink-3">requested by {device.client_name}</p>
      </div>
      <div className="grid grid-cols-2 gap-2">
        <button
          type="button"
          disabled={pending}
          onClick={() => decide(false)}
          className="pressable h-10 rounded-lg border border-line bg-surface text-[14px] text-ink-2 hover:text-ink disabled:opacity-60"
        >
          Deny
        </button>
        <button
          type="button"
          disabled={pending}
          onClick={() => decide(true)}
          className="pressable h-10 rounded-lg bg-ink font-medium text-[14px] text-bg disabled:opacity-60"
        >
          {pending ? "Working…" : "Approve"}
        </button>
      </div>
      {result && !result.ok ? (
        <p role="alert" className="text-[13px] text-accent">
          {result.message}
        </p>
      ) : null}
    </div>
  );
}
