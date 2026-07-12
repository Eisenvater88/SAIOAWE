use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use cron::Schedule as CronSchedule;
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use crate::engine::Engine;
use crate::models::{now_rfc3339, Schedule};

/// Accepts classic 5-field cron by prepending a seconds field, then
/// validates the expression. Returns the normalized 6/7-field form.
pub fn normalize_cron(expr: &str) -> Result<String> {
    let fields: Vec<&str> = expr.split_whitespace().collect();
    let normalized = if fields.len() == 5 {
        format!("0 {}", fields.join(" "))
    } else {
        fields.join(" ")
    };
    CronSchedule::from_str(&normalized)
        .with_context(|| format!("invalid cron expression '{expr}'"))?;
    Ok(normalized)
}

/// Polls the schedule table and triggers workflow runs whose cron expression
/// fired since the last tick. Missed fires (e.g. server was down) collapse
/// into a single run.
pub async fn run_scheduler(engine: Arc<Engine>) {
    let mut last_check: DateTime<Utc> = Utc::now();
    let mut ticker = tokio::time::interval(Duration::from_secs(15));
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        ticker.tick().await;
        let now = Utc::now();
        let schedules: Vec<Schedule> = match engine.db.list("schedules") {
            Ok(s) => s,
            Err(e) => {
                tracing::error!("scheduler: loading schedules failed: {e:#}");
                continue;
            }
        };
        for mut schedule in schedules {
            if !schedule.enabled {
                continue;
            }
            let Ok(normalized) = normalize_cron(&schedule.cron) else {
                tracing::warn!("scheduler: schedule {} has invalid cron '{}'", schedule.id, schedule.cron);
                continue;
            };
            let cron = CronSchedule::from_str(&normalized).expect("validated above");
            let due = cron
                .after(&last_check)
                .take_while(|t| *t <= now)
                .next()
                .is_some();
            if !due {
                continue;
            }
            tracing::info!(
                "scheduler: triggering workflow {} (schedule {})",
                schedule.workflow_id,
                schedule.id
            );
            match engine.start_run(&schedule.workflow_id, "schedule", schedule.input.clone()) {
                Ok(_) => {
                    schedule.last_run_at = Some(now_rfc3339());
                    if let Err(e) = engine.db.put("schedules", &schedule.id.clone(), &schedule) {
                        tracing::error!("scheduler: updating schedule failed: {e:#}");
                    }
                }
                Err(e) => tracing::error!(
                    "scheduler: starting workflow {} failed: {e:#}",
                    schedule.workflow_id
                ),
            }
        }
        last_check = now;
    }
}
