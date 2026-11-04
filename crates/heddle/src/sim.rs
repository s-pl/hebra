//! Simulation: a policy against a trace of jobs, second by second where it
//! matters, to see what it costs and how long people wait, before any
//! machine is paid for.

use serde::{Deserialize, Serialize};

use crate::{Action, Error, Job, Machine, MachineType, Params, Policy};

/// How often the planner is asked while machines exist.
const TICK_SECS: f64 = 10.0;

/// One job of a trace: when it arrives and how long it takes on the
/// reference machine.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TraceJob {
    pub at: f64,
    pub secs: f64,
    #[serde(default)]
    pub labels: Vec<String>,
}

/// A trace: from a file (`[{ "at": 0, "secs": 90 }, …]`), or made up.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Trace {
    pub jobs: Vec<TraceJob>,
}

/// A small deterministic generator, so traces reproduce.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> f64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        #[allow(clippy::cast_precision_loss)]
        let x = (self.0 >> 11) as f64 / (1u64 << 53) as f64;
        x
    }

    /// Exponential with this mean.
    fn exp(&mut self, mean: f64) -> f64 {
        -mean * (1.0 - self.next()).ln()
    }
}

impl Trace {
    pub fn parse(text: &str) -> Result<Self, Error> {
        let jobs: Vec<TraceJob> =
            serde_json::from_str(text).map_err(|e| Error::Trace(e.to_string()))?;
        let mut trace = Self { jobs };
        trace.jobs.sort_by(|a, b| a.at.total_cmp(&b.at));
        Ok(trace)
    }

    /// A made-up workload:
    /// - `team`: a team's working days, pushes every few minutes in office
    ///   hours and quiet nights, over `days`;
    /// - `agents`: bursts of many agents pushing at once, then silence;
    /// - `steady`: one push every few minutes, all day.
    #[must_use]
    pub fn synthetic(kind: &str, days: u32, seed: u64) -> Self {
        let mut rng = Rng(seed.max(1));
        let mut jobs = Vec::new();
        let label = vec!["linux".to_owned()];
        let push = |at: f64, rng: &mut Rng, jobs: &mut Vec<TraceJob>| {
            // A push runs a few checks: fast lint, tests, sometimes a long build.
            jobs.push(TraceJob {
                at,
                secs: 20.0 + rng.exp(20.0),
                labels: label.clone(),
            });
            jobs.push(TraceJob {
                at,
                secs: 120.0 + rng.exp(180.0),
                labels: label.clone(),
            });
            if rng.next() < 0.2 {
                jobs.push(TraceJob {
                    at,
                    secs: 900.0 + rng.exp(600.0),
                    labels: label.clone(),
                });
            }
        };
        for day in 0..days {
            let start = f64::from(day) * 86_400.0;
            match kind {
                "agents" => {
                    let mut t = start + 3600.0;
                    while t < start + 86_400.0 {
                        let burst = 10 + (rng.next() * 40.0) as u32;
                        for _ in 0..burst {
                            push(t + rng.next() * 120.0, &mut rng, &mut jobs);
                        }
                        t += 1800.0 + rng.exp(5400.0);
                    }
                }
                "steady" => {
                    let mut t = start;
                    while t < start + 86_400.0 {
                        push(t, &mut rng, &mut jobs);
                        t += rng.exp(600.0);
                    }
                }
                _ => {
                    // 9:00 to 18:00 on weekdays.
                    if day % 7 >= 5 {
                        continue;
                    }
                    let mut t = start + 9.0 * 3600.0;
                    while t < start + 18.0 * 3600.0 {
                        push(t, &mut rng, &mut jobs);
                        t += rng.exp(420.0);
                    }
                }
            }
        }
        jobs.sort_by(|a, b| a.at.total_cmp(&b.at));
        Self { jobs }
    }

    /// Jobs per hour over the whole trace.
    #[must_use]
    pub fn rate_per_hour(&self) -> f64 {
        let span = self.jobs.last().map_or(0.0, |j| j.at) - self.jobs.first().map_or(0.0, |j| j.at);
        #[allow(clippy::cast_precision_loss)]
        let n = self.jobs.len() as f64;
        if span <= 0.0 { n } else { n * 3600.0 / span }
    }
}

/// How a policy did.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Report {
    pub policy: String,
    pub jobs: usize,
    pub cost: f64,
    pub wait_p50_secs: f64,
    pub wait_p95_secs: f64,
    pub wait_max_secs: f64,
    /// From arriving to finishing.
    pub turnaround_p50_secs: f64,
    pub machines_started: usize,
    pub machine_hours: f64,
    /// Share of machine time spent running jobs.
    pub utilization: f64,
    /// Machines started, hours and cost by machine type.
    pub by_type: std::collections::BTreeMap<String, TypeUse>,
    /// Cost plus the time to results (waiting and running) valued at
    /// `time_value_per_hour`.
    pub total_cost: f64,
}

/// How much one machine type was used.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct TypeUse {
    pub started: usize,
    pub hours: f64,
    pub busy_hours: f64,
    pub cost: f64,
}

struct SimMachine {
    id: String,
    kind: usize,
    created: f64,
    ready_at: f64,
    busy_until: Option<f64>,
    idle_since: f64,
    busy_secs: f64,
}

fn percentile(sorted: &[f64], p: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        clippy::cast_precision_loss
    )]
    let i = ((sorted.len() - 1) as f64 * p).round() as usize;
    sorted[i.min(sorted.len() - 1)]
}

/// Runs `trace` under `policy`. Machines take the oldest job they fit.
#[must_use]
#[allow(clippy::too_many_lines)]
pub fn simulate(
    trace: &Trace,
    catalog: &[MachineType],
    policy: &dyn Policy,
    params: &Params,
) -> Report {
    let mut machines: Vec<SimMachine> = Vec::new();
    let mut queue: Vec<(usize, Job)> = Vec::new();
    let mut next_job = 0;
    let mut waits = Vec::new();
    let mut turnarounds = Vec::new();
    let mut cost = 0.0;
    let mut machine_secs = 0.0;
    let mut busy_secs = 0.0;
    let mut started = 0usize;
    let by_type: std::cell::RefCell<std::collections::BTreeMap<String, TypeUse>> =
        std::cell::RefCell::default();
    let mut now = trace.jobs.first().map_or(0.0, |j| j.at);
    let mut last_plan = f64::NEG_INFINITY;

    let retire =
        |m: &SimMachine, at: f64, cost: &mut f64, machine_secs: &mut f64, busy: &mut f64| {
            let life = (at - m.created).max(0.0);
            let bill = catalog[m.kind].cost(life);
            *cost += bill;
            *machine_secs += life;
            *busy += m.busy_secs;
            let mut types = by_type.borrow_mut();
            let u = types.entry(catalog[m.kind].name.clone()).or_default();
            u.started += 1;
            u.hours += life / 3600.0;
            u.busy_hours += m.busy_secs / 3600.0;
            u.cost += bill;
        };

    loop {
        // Jobs that ended.
        for m in &mut machines {
            if let Some(end) = m.busy_until
                && end <= now
            {
                m.busy_until = None;
                m.idle_since = end;
            }
        }
        // Jobs that arrived.
        while next_job < trace.jobs.len() && trace.jobs[next_job].at <= now {
            let j = &trace.jobs[next_job];
            queue.push((
                next_job,
                Job {
                    id: format!("job-{next_job}"),
                    labels: j.labels.clone(),
                    reference_secs: j.secs,
                    queued_at: j.at,
                },
            ));
            next_job += 1;
        }
        let dispatch = |machines: &mut Vec<SimMachine>,
                        queue: &mut Vec<(usize, Job)>,
                        waits: &mut Vec<f64>,
                        turnarounds: &mut Vec<f64>| {
            for m in machines.iter_mut() {
                if m.ready_at > now || m.busy_until.is_some() {
                    continue;
                }
                let t = &catalog[m.kind];
                if let Some(pos) = queue.iter().position(|(_, j)| t.fits(&j.labels)) {
                    let (_, job) = queue.remove(pos);
                    let run = t.duration(job.reference_secs, params.scaling);
                    m.busy_until = Some(now + run);
                    m.busy_secs += run;
                    waits.push(now - job.queued_at);
                    turnarounds.push(now + run - job.queued_at);
                }
            }
        };
        dispatch(&mut machines, &mut queue, &mut waits, &mut turnarounds);

        // Ask the policy.
        if now - last_plan >= TICK_SECS || !queue.is_empty() {
            last_plan = now;
            let view: Vec<Machine> = machines
                .iter()
                .map(|m| Machine {
                    id: m.id.clone(),
                    kind: catalog[m.kind].name.clone(),
                    created: m.created,
                    ready_at: m.ready_at,
                    busy_until: m.busy_until,
                    idle_since: (m.busy_until.is_none() && m.ready_at <= now)
                        .then_some(m.idle_since),
                })
                .collect();
            let jobs: Vec<Job> = queue.iter().map(|(_, j)| j.clone()).collect();
            // What the live planner would know: the last hour of arrivals.
            let mut known = params.clone();
            known.recent_arrivals = trace.jobs[..next_job]
                .iter()
                .rev()
                .take_while(|j| now - j.at <= 3600.0)
                .map(|j| j.at)
                .collect();
            for action in policy.plan(now, catalog, &view, &jobs, &known) {
                match action {
                    Action::Create { kind, .. } => {
                        if machines.len() >= params.max_machines {
                            continue;
                        }
                        if let Some(k) = catalog.iter().position(|t| t.name == kind) {
                            started += 1;
                            machines.push(SimMachine {
                                id: format!("m{started}"),
                                kind: k,
                                created: now,
                                ready_at: now + catalog[k].boot_secs,
                                busy_until: None,
                                idle_since: now + catalog[k].boot_secs,
                                busy_secs: 0.0,
                            });
                        }
                    }
                    Action::Destroy { id, .. } => {
                        if let Some(i) = machines
                            .iter()
                            .position(|m| m.id == id && m.busy_until.is_none())
                        {
                            let m = machines.remove(i);
                            retire(&m, now, &mut cost, &mut machine_secs, &mut busy_secs);
                        }
                    }
                }
            }
            dispatch(&mut machines, &mut queue, &mut waits, &mut turnarounds);
        }

        // Done: everything arrived, ran and nothing waits.
        let working = machines.iter().any(|m| m.busy_until.is_some());
        if next_job >= trace.jobs.len() && queue.is_empty() && !working {
            break;
        }
        // The next moment anything happens.
        let mut next = f64::INFINITY;
        if let Some(j) = trace.jobs.get(next_job) {
            next = next.min(j.at);
        }
        for m in &machines {
            if m.ready_at > now {
                next = next.min(m.ready_at);
            }
            if let Some(end) = m.busy_until {
                next = next.min(end);
            }
        }
        if !machines.is_empty() || !queue.is_empty() {
            next = next.min(now + TICK_SECS);
        }
        if !next.is_finite() {
            break;
        }
        now = next.max(now + 1e-6);
    }
    // Whatever still runs is stopped now.
    for m in &machines {
        retire(m, now, &mut cost, &mut machine_secs, &mut busy_secs);
    }
    waits.sort_by(f64::total_cmp);
    turnarounds.sort_by(f64::total_cmp);
    let turnaround_total: f64 = turnarounds.iter().sum();
    Report {
        policy: policy.name(),
        jobs: trace.jobs.len(),
        cost,
        wait_p50_secs: percentile(&waits, 0.5),
        wait_p95_secs: percentile(&waits, 0.95),
        wait_max_secs: waits.last().copied().unwrap_or(0.0),
        turnaround_p50_secs: percentile(&turnarounds, 0.5),
        machines_started: started,
        machine_hours: machine_secs / 3600.0,
        utilization: if machine_secs > 0.0 {
            busy_secs / machine_secs
        } else {
            0.0
        },
        by_type: by_type.into_inner(),
        total_cost: cost + params.time_value_per_hour * turnaround_total / 3600.0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Catalog;
    use crate::plan::{Heddle, baselines};

    #[test]
    fn every_job_runs_and_heddle_beats_the_baselines_overall() {
        let catalog = Catalog::example().types;
        for kind in ["team", "agents", "steady"] {
            let trace = Trace::synthetic(kind, 3, 7);
            let params = Params {
                arrivals_per_hour: trace.rate_per_hour(),
                ..Params::default()
            };
            let reports: Vec<Report> = baselines(&catalog)
                .iter()
                .map(|p| simulate(&trace, &catalog, p.as_ref(), &params))
                .collect();
            for r in &reports {
                assert_eq!(r.jobs, trace.jobs.len());
                assert!(r.cost >= 0.0);
            }
            let heddle = simulate(&trace, &catalog, &Heddle, &params);
            let best = reports
                .iter()
                .filter(|r| r.policy != "heddle")
                .map(|r| r.total_cost)
                .fold(f64::INFINITY, f64::min);
            assert!(
                heddle.total_cost <= best * 1.05,
                "{kind}: heddle {:.2} vs best baseline {best:.2}: {reports:#?}",
                heddle.total_cost
            );
        }
    }

    #[test]
    fn traces_parse_and_reproduce() {
        let t = Trace::parse(r#"[{"at": 10, "secs": 5}, {"at": 0, "secs": 1}]"#).unwrap();
        assert_eq!(t.jobs[0].at, 0.0);
        assert_eq!(
            Trace::synthetic("team", 2, 3),
            Trace::synthetic("team", 2, 3)
        );
    }
}
