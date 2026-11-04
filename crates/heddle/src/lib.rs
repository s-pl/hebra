//! Heddle™: build intelligence for Hebra.
//!
//! In a loom, the heddles decide which threads rise on each pass. Heddle
//! decides which checks run, where, and on how many machines:
//!
//! - the server skips what does not need to run: checks whose inputs did
//!   not change are reused, checks whose paths did not change are skipped,
//!   checks wait for the ones they need (see `hebra-server`'s `checks`);
//! - this crate decides which machines to start and stop for the queue that
//!   is left ([`plan`]), and measures any policy against a trace of real or
//!   made-up work ([`sim`]) before it spends money.
//!
//! The planner weighs money against people's time: every option costs what
//! the machine bills plus what the wait and the run are worth
//! ([`Params::time_value_per_hour`]). It never boots a machine that would be
//! ready later than one already running frees up, uses hours already paid
//! for, and stops machines just before they would bill again.
//!
//! Providers (Docker on one computer today; cloud APIs later) only create,
//! list and destroy machines: what to do is decided here, so a policy
//! proven in simulation behaves the same on real machines.

pub mod plan;
pub mod sim;

use serde::{Deserialize, Serialize};

/// Jobs are timed on a machine with this many CPUs.
pub const REFERENCE_CPUS: f64 = 2.0;

/// How a provider bills a machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Billing {
    /// By the second, with a minimum (`min_billed_secs`).
    Second,
    /// By the started hour.
    Hour,
}

/// A kind of machine a provider offers.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MachineType {
    pub name: String,
    pub cpus: u32,
    #[serde(default)]
    pub memory_gb: u32,
    /// In any currency, as long as all types use the same one.
    pub price_per_hour: f64,
    #[serde(default = "per_hour")]
    pub billing: Billing,
    #[serde(default)]
    pub min_billed_secs: u64,
    /// From asking for the machine to its runner taking work.
    pub boot_secs: f64,
    /// What checks may ask for (`labels = ["linux", "gpu"]`).
    #[serde(default)]
    pub labels: Vec<String>,
    /// Most machines of this type at once.
    pub max: Option<usize>,
}

const fn per_hour() -> Billing {
    Billing::Hour
}

impl MachineType {
    /// Seconds a job takes here, from its time on the reference machine.
    /// `scaling` 0 means more CPUs do not help, 1 means they help fully.
    #[must_use]
    pub fn duration(&self, reference_secs: f64, scaling: f64) -> f64 {
        let cpus = f64::from(self.cpus.max(1));
        reference_secs * (REFERENCE_CPUS / cpus).powf(scaling)
    }

    #[must_use]
    pub fn fits(&self, labels: &[String]) -> bool {
        labels.iter().all(|l| self.labels.contains(l))
    }

    /// What keeping the machine for `secs` costs, rounded as it is billed.
    #[must_use]
    pub fn cost(&self, secs: f64) -> f64 {
        #[allow(clippy::cast_precision_loss)]
        let billed = match self.billing {
            Billing::Second => secs.max(self.min_billed_secs as f64),
            Billing::Hour => (secs / 3600.0).ceil().max(1.0) * 3600.0,
        };
        self.price_per_hour * billed / 3600.0
    }

    /// When the hour being paid for ends (hourly billing).
    #[must_use]
    pub fn paid_until(&self, created: f64, now: f64) -> f64 {
        match self.billing {
            Billing::Hour => created + ((now - created).max(1.0) / 3600.0).ceil() * 3600.0,
            #[allow(clippy::cast_precision_loss)]
            Billing::Second => created + (self.min_billed_secs as f64).max(now - created),
        }
    }
}

/// A catalog of machine types, from a TOML file:
///
/// ```toml
/// [[type]]
/// name = "small"
/// cpus = 2
/// price_per_hour = 0.01
/// billing = "hour"
/// boot_secs = 30
/// labels = ["linux"]
/// ```
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Catalog {
    #[serde(rename = "type", default)]
    pub types: Vec<MachineType>,
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("catalog: {0}")]
    Catalog(String),
    #[error("trace: {0}")]
    Trace(String),
}

impl Catalog {
    pub fn parse(text: &str) -> Result<Self, Error> {
        let catalog: Self = toml::from_str(text).map_err(|e| Error::Catalog(e.to_string()))?;
        if catalog.types.is_empty() {
            return Err(Error::Catalog("no machine types".to_owned()));
        }
        Ok(catalog)
    }

    /// An example catalog for simulations and local runs. The prices are
    /// illustrative, of the order of small cloud servers; put your
    /// provider's in a file for real decisions.
    #[must_use]
    pub fn example() -> Self {
        let t = |name: &str, cpus, memory_gb, price_per_hour, billing, boot_secs| MachineType {
            name: name.to_owned(),
            cpus,
            memory_gb,
            price_per_hour,
            billing,
            min_billed_secs: 60,
            boot_secs,
            labels: vec!["linux".to_owned()],
            max: None,
        };
        Self {
            types: vec![
                t("hourly-2", 2, 4, 0.006, Billing::Hour, 35.0),
                t("hourly-8", 8, 16, 0.03, Billing::Hour, 40.0),
                t("second-2", 2, 4, 0.02, Billing::Second, 25.0),
                t("second-16", 16, 32, 0.16, Billing::Second, 30.0),
            ],
        }
    }
}

/// A machine as the planner sees it, times in seconds on one clock.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Machine {
    pub id: String,
    pub kind: String,
    pub created: f64,
    /// When it can take work (in the past once it can).
    pub ready_at: f64,
    /// When its current job should end, if it has one.
    pub busy_until: Option<f64>,
    /// Since when it has had nothing to do.
    pub idle_since: Option<f64>,
}

/// A check waiting for a machine.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Job {
    pub id: String,
    #[serde(default)]
    pub labels: Vec<String>,
    /// Expected seconds on the reference machine (Heddle's history).
    pub reference_secs: f64,
    pub queued_at: f64,
}

/// What a policy is allowed and asked to do.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Params {
    pub max_machines: usize,
    /// Most the running machines may cost per hour together.
    pub max_spend_per_hour: f64,
    /// What an hour of someone waiting for a check is worth, in the
    /// catalog's currency. Zero optimizes cost only.
    pub time_value_per_hour: f64,
    /// Waits shorter than this never start a machine.
    pub latency_target_secs: f64,
    /// Waits longer than this start a machine whatever it costs (within
    /// the limits on machines and spend).
    pub max_wait_secs: f64,
    /// Per-second machines stop after this long idle.
    pub idle_grace_secs: f64,
    /// Hourly machines stop this long before the next hour starts.
    pub teardown_margin_secs: f64,
    /// Jobs per hour expected (from recent history).
    pub arrivals_per_hour: f64,
    /// How much more CPUs speed jobs up (0 to 1).
    pub scaling: f64,
    /// Idle machines kept ready whatever it costs.
    pub min_idle: usize,
    /// When recent jobs arrived (the last hour or so), for the forecast:
    /// how often work comes, and in bursts of how many.
    #[serde(default)]
    pub recent_arrivals: Vec<f64>,
}

impl Default for Params {
    fn default() -> Self {
        Self {
            max_machines: 20,
            max_spend_per_hour: 10.0,
            time_value_per_hour: 3.0,
            latency_target_secs: 20.0,
            max_wait_secs: 90.0,
            idle_grace_secs: 120.0,
            teardown_margin_secs: 180.0,
            arrivals_per_hour: 0.0,
            scaling: 0.6,
            min_idle: 0,
            recent_arrivals: Vec::new(),
        }
    }
}

/// What to optimize for, as a starting point for [`Params`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Goal {
    /// The smallest bill; waits up to five minutes.
    Cheap,
    /// Short waits without paying for idle machines.
    Balanced,
    /// Results as soon as possible.
    Fast,
}

impl Goal {
    pub fn parse(text: &str) -> Option<Self> {
        Some(match text {
            "cheap" => Self::Cheap,
            "balanced" => Self::Balanced,
            "fast" => Self::Fast,
            _ => return None,
        })
    }

    #[must_use]
    pub fn params(self) -> Params {
        let (time_value_per_hour, max_wait_secs, latency_target_secs) = match self {
            Self::Cheap => (0.0, 300.0, 60.0),
            Self::Balanced => (3.0, 90.0, 20.0),
            Self::Fast => (30.0, 20.0, 5.0),
        };
        Params {
            time_value_per_hour,
            max_wait_secs,
            latency_target_secs,
            ..Params::default()
        }
    }
}

/// What a policy decides.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "lowercase")]
pub enum Action {
    Create {
        kind: String,
        /// The job it is started for (it may end up taking another).
        job: Option<String>,
        reason: String,
    },
    Destroy {
        id: String,
        reason: String,
    },
}

/// A way of deciding which machines to run.
pub trait Policy {
    fn name(&self) -> String;
    fn plan(
        &self,
        now: f64,
        catalog: &[MachineType],
        machines: &[Machine],
        queue: &[Job],
        params: &Params,
    ) -> Vec<Action>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn billing_rounds_as_providers_do() {
        let c = Catalog::example();
        let hourly = &c.types[0];
        assert!((hourly.cost(10.0) - 0.006).abs() < 1e-9);
        assert!((hourly.cost(3601.0) - 0.012).abs() < 1e-9);
        let second = &c.types[2];
        assert!((second.cost(30.0) - 0.02 * 60.0 / 3600.0).abs() < 1e-9);
        assert!(c.types[3].duration(100.0, 1.0) < 15.0);
        assert!((hourly.paid_until(0.0, 100.0) - 3600.0).abs() < 1e-9);
        let parsed = Catalog::parse(
            "[[type]]\nname = \"a\"\ncpus = 4\nprice_per_hour = 0.1\nbilling = \"second\"\nboot_secs = 10\n",
        )
        .unwrap();
        assert_eq!(parsed.types[0].billing, Billing::Second);
        assert!(Catalog::parse("").is_err());
    }
}
