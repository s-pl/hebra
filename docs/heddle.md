# Heddle™

Heddle decides what runs, where, and on how many machines. It has two parts:

1. **Reuse.** A check whose inputs match a run that passed is not run again
   (see [checks](ci.md)). This is always on.
2. **The fleet.** For an owner who sets it up, the server starts runner
   machines at a cloud provider when checks wait, and deletes them when
   keeping them costs more than the next wait would.

## Setting up a fleet

In the hub: Settings → the owner → **Heddle** → Settings. Or with the API
(`PUT /accounts/{owner}/fleet`):

| Setting | |
|---|---|
| Provider | `hetzner`, `digitalocean`, `vultr`, or `docker` (containers on the server's own machine, to try it without a bill). |
| API token | The provider's token, with write access. Stored sealed (XChaCha20-Poly1305); never shown again. |
| Goal | `cheap`, `balanced` or `fast`: how much an hour of people waiting is worth against an hour of machine. |
| Most machines | At once, all types together. |
| Most spend per hour | All machines together. |
| Always ready | Idle machines kept for the next check. |
| Labels | Every machine of the fleet has them; checks ask for labels. |
| Region, image, SSH keys, billing, hb version, boot timeout | Per provider; the defaults work. |

Before switching it on, **Replay my checks** runs each policy on the owner's
own finished checks of the last days, with the provider's live prices, and
shows the machine cost, the time checks waited for a machine and how much of
the machine time went to checks. Nothing is started.

## What the orchestrator does

One server replica at a time (a lease), every few seconds, for each fleet:

1. **Reconciles** with the provider. Machines the provider no longer has are
   marked gone. A machine with the fleet's tag that the server does not know
   is deleted, so nothing bills unseen. A create that never finished fails.
2. **Follows each machine's runner.** A machine is ready once its runner
   connects. One whose runner does not connect within the boot timeout is
   deleted.
3. **Plans.** Heddle's planner looks at the queue (labels, how long each
   check usually takes on this owner's history), the machines and what they
   are doing, the provider's machine types and prices, and the goal:
   - a check waits for a busy machine when it frees up sooner than a new one
     would boot;
   - a long check gets a bigger machine when the time saved is worth more
     than the price;
   - a machine billed by the hour finishes the hour already paid for, and
     stays into the next one only if recent pushes say work is coming;
   - nothing goes over the machine and spending limits.
4. **Acts.** A new machine gets a one-time runner token in its first-boot
   script (cloud-init), installs `hb`, registers its runner and starts taking
   checks. A machine Heddle stops is deleted at the provider, with its runner.
5. **Bills.** What each machine cost so far, rounded as the provider bills
   (by the hour, or by the second with a minimum).

Every start, stop and problem is written to the fleet's log with its reason,
for example `cx22 (2 CPU, 0.0062/h, billed by the hour): acme/app test would
wait 95s`. The hub shows it live, with a timeline of the machines and the
checks they ran.

## On your computer

The same planner runs locally, with Docker containers standing in for
virtual machines:

```sh
hb heddle simulate --workload team --goal balanced     # every policy on a made-up week
hb heddle up --owner acme --server https://hebra.example.com --goal balanced --max-machines 4
hb heddle down --owner acme
```

`--catalog` takes a TOML file of machine types (CPUs, memory, price, billing,
boot time, labels) to try your own prices.

## Not there yet

- GPU clouds and spot or preemptible machines.
- Machines of one fleet in several regions or providers at once.
