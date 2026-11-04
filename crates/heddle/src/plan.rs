//! Policies: the common ones a CI service uses, to compare against, and
//! Heddle's.

use std::collections::HashSet;

use crate::{Action, Billing, Job, Machine, MachineType, Params, Policy};

fn kind<'a>(catalog: &'a [MachineType], name: &str) -> Option<&'a MachineType> {
    catalog.iter().find(|t| t.name == name)
}

fn is_idle(m: &Machine, now: f64) -> bool {
    m.ready_at <= now && m.busy_until.is_none()
}

/// A fixed fleet, always on: a self-hosted runner box, or a few.
pub struct AlwaysOn {
    pub machines: usize,
    pub kind: String,
}

impl Policy for AlwaysOn {
    fn name(&self) -> String {
        format!("always-on ×{} {}", self.machines, self.kind)
    }

    fn plan(
        &self,
        _: f64,
        _: &[MachineType],
        machines: &[Machine],
        _: &[Job],
        _: &Params,
    ) -> Vec<Action> {
        (machines.len()..self.machines)
            .map(|_| Action::Create {
                kind: self.kind.clone(),
                job: None,
                reason: "fleet".to_owned(),
            })
            .collect()
    }
}

/// A fresh machine for every job, gone as soon as it is idle (how hosted
/// CI services run jobs).
pub struct PerJob {
    pub kind: String,
}

impl Policy for PerJob {
    fn name(&self) -> String {
        format!("one machine per job ({})", self.kind)
    }

    fn plan(
        &self,
        now: f64,
        _: &[MachineType],
        machines: &[Machine],
        queue: &[Job],
        params: &Params,
    ) -> Vec<Action> {
        let mut out = Vec::new();
        let booting = machines.iter().filter(|m| m.ready_at > now).count();
        let room = params.max_machines.saturating_sub(machines.len());
        for job in queue.iter().skip(booting).take(room) {
            out.push(Action::Create {
                kind: self.kind.clone(),
                job: Some(job.id.clone()),
                reason: "a job is waiting".to_owned(),
            });
        }
        for m in machines.iter().filter(|m| is_idle(m, now)) {
            out.push(Action::Destroy {
                id: m.id.clone(),
                reason: "done".to_owned(),
            });
        }
        out
    }
}

/// Start machines while jobs wait; stop them after a while idle (the usual
/// autoscaler).
pub struct IdleTimeout {
    pub kind: String,
    pub idle_secs: f64,
}

impl Policy for IdleTimeout {
    fn name(&self) -> String {
        format!(
            "autoscale, stop after {}s idle ({})",
            self.idle_secs, self.kind
        )
    }

    fn plan(
        &self,
        now: f64,
        _: &[MachineType],
        machines: &[Machine],
        queue: &[Job],
        params: &Params,
    ) -> Vec<Action> {
        let mut out = Vec::new();
        let free = machines.iter().filter(|m| m.busy_until.is_none()).count();
        let room = params.max_machines.saturating_sub(machines.len());
        for job in queue.iter().skip(free).take(room) {
            out.push(Action::Create {
                kind: self.kind.clone(),
                job: Some(job.id.clone()),
                reason: "a job is waiting".to_owned(),
            });
        }
        for m in machines.iter().filter(|m| is_idle(m, now)) {
            if m.idle_since.is_some_and(|s| now - s >= self.idle_secs) {
                out.push(Action::Destroy {
                    id: m.id.clone(),
                    reason: "idle".to_owned(),
                });
            }
        }
        out
    }
}

/// Heddle's policy.
///
/// For each waiting job, oldest first, it compares waiting for a machine
/// that is running (or booting) with starting a new one of each type, by
/// money plus the time it costs people, and takes the cheapest. A new
/// machine is never started when an existing one frees up before it would
/// boot, nor for waits under the latency target. When more work is
/// expected during its life, a machine is priced by the time it runs rather
/// than by its first billing period, since it will be used again.
///
/// An idle machine is kept while what it is expected to save is worth more
/// than what it costs: work arrives in bursts (a push runs several checks),
/// so the `n`-th idle machine is worth keeping when bursts of `n` or more
/// jobs come often enough that a boot saved, in waiting and in billing,
/// pays for it. Hourly machines cost nothing more inside a paid hour, so
/// they are only reconsidered before the next one starts. The forecast is
/// the last hour of arrivals, so machines stay warm in working hours and go
/// at night.
pub struct Heddle;

/// A machine's place on the planner's timeline.
struct Slot<'a> {
    machine: Option<&'a Machine>,
    kind: &'a MachineType,
    free_at: f64,
}

/// Arrivals closer than this belong to one burst.
const BURST_GAP_SECS: f64 = 15.0;
/// The forecast looks back this far.
const WINDOW_SECS: f64 = 3600.0;

/// Bursts of recent arrivals: their sizes, within the window.
fn bursts(params: &Params, now: f64) -> (Vec<usize>, f64) {
    let mut times: Vec<f64> = params
        .recent_arrivals
        .iter()
        .copied()
        .filter(|t| *t <= now && now - t <= WINDOW_SECS)
        .collect();
    times.sort_by(f64::total_cmp);
    let mut sizes = Vec::new();
    let mut last = f64::NEG_INFINITY;
    for t in &times {
        if t - last > BURST_GAP_SECS {
            sizes.push(0);
        }
        if let Some(s) = sizes.last_mut() {
            *s += 1;
        }
        last = *t;
    }
    // Always the whole window: a burst a few minutes ago is not a rate.
    (sizes, WINDOW_SECS)
}

impl Heddle {
    fn time_cost(params: &Params, secs: f64) -> f64 {
        params.time_value_per_hour * secs / 3600.0
    }

    /// Jobs per second expected: from recent arrivals, else the configured
    /// rate.
    fn rate(params: &Params, now: f64) -> f64 {
        let (sizes, span) = bursts(params, now);
        if params.recent_arrivals.is_empty() {
            return params.arrivals_per_hour / 3600.0;
        }
        #[allow(clippy::cast_precision_loss)]
        let jobs = sizes.iter().sum::<usize>() as f64;
        jobs / span
    }

    /// Bursts per second that need at least `n` machines at once.
    fn burst_rate(params: &Params, now: f64, n: usize) -> f64 {
        if params.recent_arrivals.is_empty() {
            // Only a rate: treat each job as its own burst.
            return if n <= 1 {
                params.arrivals_per_hour / 3600.0
            } else {
                0.0
            };
        }
        let (sizes, span) = bursts(params, now);
        #[allow(clippy::cast_precision_loss)]
        let count = sizes.iter().filter(|&&s| s >= n).count() as f64;
        count / span
    }
}

impl Policy for Heddle {
    fn name(&self) -> String {
        "heddle".to_owned()
    }

    #[allow(clippy::too_many_lines)]
    fn plan(
        &self,
        now: f64,
        catalog: &[MachineType],
        machines: &[Machine],
        queue: &[Job],
        params: &Params,
    ) -> Vec<Action> {
        let mut out = Vec::new();
        let mut slots: Vec<Slot<'_>> = machines
            .iter()
            .filter_map(|m| {
                Some(Slot {
                    machine: Some(m),
                    kind: kind(catalog, &m.kind)?,
                    free_at: m.ready_at.max(m.busy_until.unwrap_or(now)).max(now),
                })
            })
            .collect();
        let mut spend: f64 = slots.iter().map(|s| s.kind.price_per_hour).sum();
        let mut used: HashSet<&str> = HashSet::new();
        let rate = Self::rate(params, now);

        let mut waiting: Vec<&Job> = queue.iter().collect();
        waiting.sort_by(|a, b| a.queued_at.total_cmp(&b.queued_at));
        for job in waiting {
            // Waiting for a machine already there.
            let existing = slots
                .iter()
                .enumerate()
                .filter(|(_, s)| s.kind.fits(&job.labels))
                .map(|(i, s)| {
                    let run = s.kind.duration(job.reference_secs, params.scaling);
                    let wait = s.free_at - now;
                    // Money: the extra time it bills (hourly machines inside
                    // a paid hour cost nothing more).
                    let money = match (s.kind.billing, s.machine) {
                        (Billing::Hour, Some(m))
                            if s.free_at + run <= s.kind.paid_until(m.created, s.free_at) =>
                        {
                            0.0
                        }
                        _ => s.kind.price_per_hour * run / 3600.0,
                    };
                    (i, wait, money + Self::time_cost(params, wait + run))
                })
                .min_by(|a, b| a.2.total_cmp(&b.2));

            // Starting one.
            let can_start = |t: &MachineType| {
                slots.len() < params.max_machines
                    && spend + t.price_per_hour <= params.max_spend_per_hour
                    && t.max.is_none_or(|max| {
                        slots.iter().filter(|s| s.kind.name == t.name).count() < max
                    })
            };
            let fresh = catalog
                .iter()
                .filter(|t| t.fits(&job.labels) && can_start(t))
                .map(|t| {
                    let run = t.duration(job.reference_secs, params.scaling);
                    let total = t.boot_secs + run;
                    // Work keeps coming (a few jobs an hour or more): an hourly
                    // machine will be reused, so its time costs what it runs
                    // divided by how busy it is expected to be. Per-second
                    // machines go when idle: they cost what they bill.
                    let money = match t.billing {
                        Billing::Hour if rate * 3600.0 >= 2.0 => {
                            let load = rate * t.duration(job.reference_secs, params.scaling);
                            #[allow(clippy::cast_precision_loss)]
                            let busy = (load / (slots.len() + 1) as f64).clamp(0.1, 1.0);
                            t.price_per_hour * total / 3600.0 / busy
                        }
                        _ => t.cost(total),
                    };
                    (t, t.boot_secs, money + Self::time_cost(params, total))
                })
                .min_by(|a, b| a.2.total_cmp(&b.2));

            let start_new = match (existing, fresh) {
                (None, Some(_)) => true,
                (Some((_, wait, cost)), Some((_, boot, new_cost))) => {
                    wait > params.latency_target_secs
                        && wait > boot
                        && (new_cost < cost || wait > params.max_wait_secs)
                }
                _ => false,
            };
            if start_new && let Some((t, _, _)) = fresh {
                let run = t.duration(job.reference_secs, params.scaling);
                slots.push(Slot {
                    machine: None,
                    kind: t,
                    free_at: now + t.boot_secs + run,
                });
                spend += t.price_per_hour;
                out.push(Action::Create {
                    kind: t.name.clone(),
                    job: Some(job.id.clone()),
                    reason: match existing {
                        Some((_, wait, _)) => format!("{} would wait {wait:.0}s", job.id),
                        None => format!("nothing can run {}", job.id),
                    },
                });
            } else if let Some((i, _, _)) = existing {
                let s = &mut slots[i];
                s.free_at += s.kind.duration(job.reference_secs, params.scaling);
                if let Some(m) = s.machine {
                    used.insert(m.id.as_str());
                }
            }
        }

        // Idle machines nothing was planned for: the cheapest to keep first.
        let mut idle: Vec<(&Machine, &MachineType)> = machines
            .iter()
            .filter(|m| is_idle(m, now) && !used.contains(m.id.as_str()))
            .filter_map(|m| Some((m, kind(catalog, &m.kind)?)))
            .collect();
        let keep_cost = |m: &Machine, t: &MachineType| match t.billing {
            Billing::Hour if t.paid_until(m.created, now) - now > params.teardown_margin_secs => {
                0.0
            }
            _ => t.price_per_hour,
        };
        idle.sort_by(|a, b| keep_cost(a.0, a.1).total_cmp(&keep_cost(b.0, b.1)));
        // Machines that will be free soon count as warm already.
        let soon = machines
            .iter()
            .filter(|m| {
                m.busy_until
                    .is_some_and(|b| b - now <= params.latency_target_secs)
            })
            .count();
        for (n, (m, t)) in idle.into_iter().enumerate() {
            if n < params.min_idle {
                continue;
            }
            // Bursts in the next hour that would need this machine too.
            let expected = Self::burst_rate(params, now, soon + n + 1) * 3600.0;
            let boot_wait = Self::time_cost(params, t.boot_secs);
            let stop = match t.billing {
                Billing::Hour => {
                    // Keeping it costs another hour. Letting it go costs an
                    // hour only if work comes (probability p), plus a boot's
                    // wait each time it does.
                    let left = t.paid_until(m.created, now) - now;
                    let p = 1.0 - (-expected).exp();
                    let keep = t.price_per_hour * (1.0 - p) <= expected * boot_wait;
                    (left <= params.teardown_margin_secs && !keep).then(|| {
                        format!("its paid hour ends in {left:.0}s and little work is coming")
                    })
                }
                Billing::Second => {
                    // Keeping it costs its price; each boot avoided saves its
                    // first billing and its wait.
                    let idle_for = m.idle_since.map_or(0.0, |s| now - s);
                    let keep = t.price_per_hour <= expected * (boot_wait + t.cost(t.boot_secs));
                    (idle_for >= params.idle_grace_secs.min(60.0) && !keep)
                        .then(|| format!("idle for {idle_for:.0}s and little work is coming"))
                }
            };
            if let Some(reason) = stop {
                out.push(Action::Destroy {
                    id: m.id.clone(),
                    reason,
                });
            }
        }
        out
    }
}

/// The policies `hb heddle simulate` compares, for a catalog.
#[must_use]
pub fn baselines(catalog: &[MachineType]) -> Vec<Box<dyn Policy>> {
    let cheapest = catalog
        .iter()
        .min_by(|a, b| a.price_per_hour.total_cmp(&b.price_per_hour))
        .map(|t| t.name.clone())
        .unwrap_or_default();
    let per_second = catalog
        .iter()
        .filter(|t| t.billing == Billing::Second)
        .min_by(|a, b| a.price_per_hour.total_cmp(&b.price_per_hour))
        .map_or_else(|| cheapest.clone(), |t| t.name.clone());
    vec![
        Box::new(AlwaysOn {
            machines: 2,
            kind: cheapest.clone(),
        }),
        Box::new(PerJob {
            kind: per_second.clone(),
        }),
        Box::new(IdleTimeout {
            kind: cheapest,
            idle_secs: 300.0,
        }),
        Box::new(Heddle),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Catalog;

    fn job(id: &str, secs: f64) -> Job {
        Job {
            id: id.to_owned(),
            labels: vec!["linux".to_owned()],
            reference_secs: secs,
            queued_at: 0.0,
        }
    }

    #[test]
    fn heddle_waits_for_a_machine_that_frees_up_before_a_new_one_boots() {
        let c = Catalog::example().types;
        let busy = Machine {
            id: "m1".to_owned(),
            kind: "hourly-2".to_owned(),
            created: 0.0,
            ready_at: 0.0,
            busy_until: Some(110.0),
            idle_since: None,
        };
        let plan = Heddle.plan(
            100.0,
            &c,
            std::slice::from_ref(&busy),
            &[job("a", 60.0)],
            &Params::default(),
        );
        assert!(plan.is_empty(), "{plan:?}");
        // A long wait starts a machine.
        let long = Machine {
            busy_until: Some(5000.0),
            ..busy
        };
        let plan = Heddle.plan(100.0, &c, &[long], &[job("a", 60.0)], &Params::default());
        assert!(matches!(plan[0], Action::Create { .. }), "{plan:?}");
    }

    #[test]
    fn heddle_uses_paid_hours_and_stops_before_the_next() {
        let c = Catalog::example().types;
        let idle = |created| Machine {
            id: "m".to_owned(),
            kind: "hourly-2".to_owned(),
            created,
            ready_at: created,
            busy_until: None,
            idle_since: Some(created + 50.0),
        };
        let p = Params::default();
        assert!(Heddle.plan(1000.0, &c, &[idle(0.0)], &[], &p).is_empty());
        let plan = Heddle.plan(3500.0, &c, &[idle(0.0)], &[], &p);
        assert!(matches!(plan[0], Action::Destroy { .. }), "{plan:?}");
        // Unless work is coming faster than a machine boots.
        let busy_day = Params {
            arrivals_per_hour: 120.0,
            ..p
        };
        assert!(
            Heddle
                .plan(3500.0, &c, &[idle(0.0)], &[], &busy_day)
                .is_empty()
        );
    }

    #[test]
    fn long_jobs_get_bigger_machines_when_time_is_worth_it() {
        let c = Catalog::example().types;
        let plan = Heddle.plan(0.0, &c, &[], &[job("build", 3600.0)], &Params::default());
        let Action::Create { kind, .. } = &plan[0] else {
            panic!("{plan:?}")
        };
        assert_ne!(
            kind, "hourly-2",
            "an hour-long build should not take the smallest machine"
        );
        let cheap = Params {
            time_value_per_hour: 0.0,
            ..Params::default()
        };
        let plan = Heddle.plan(0.0, &c, &[], &[job("build", 3600.0)], &cheap);
        let Action::Create { kind, .. } = &plan[0] else {
            panic!("{plan:?}")
        };
        assert_eq!(kind, "hourly-2");
    }
}
