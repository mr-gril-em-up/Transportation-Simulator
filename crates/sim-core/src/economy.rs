//! Money: capital charges when lines are built, monthly settlement of fares,
//! operating cost, operating assistance and the local transit sales tax.
//!
//! Funding follows Collins's decision (D30): the network is paid for by
//! federal and state grants plus a local tax base. Bonds, debt service and
//! bankruptcy are F26 and not here yet, so cash may go negative.
//! Math and sources: docs/MODEL.md section 10.

use crate::model::LineResult;
use crate::params::{ModeDefaults, Params};
use serde::Serialize;

/// Capital cost of a line as built: guideway per km plus its vehicle fleet.
pub fn capital_cost(line: &LineResult, d: &ModeDefaults) -> f64 {
    d.capital_per_km * line.length_km + line.vehicles_required * d.cars_per_vehicle * d.vehicle_cost
}

/// Federal capital grant for `spend` dollars on a line expected to carry
/// `daily_boardings`: `federal_capital_share` of the spend when the line's
/// total capital cost per daily boarding passes the cost-effectiveness gate, else 0.
pub fn federal_grant(spend: f64, total_capital: f64, daily_boardings: f64, p: &Params) -> f64 {
    if spend <= 0.0 || daily_boardings <= 0.0 {
        return 0.0;
    }
    if total_capital / daily_boardings <= p.federal_max_capital_per_daily_rider {
        spend * p.federal_capital_share
    } else {
        0.0
    }
}

/// One month's money in and out. Positive numbers are amounts; `net` is the change in cash.
#[derive(Debug, Clone, Default, Serialize)]
pub struct MonthStatement {
    /// Game month index (30-day months, from `Clock::month`).
    pub month: u64,
    /// Fares collected.
    pub fare_revenue: f64,
    /// Cost of running service.
    pub operating_cost: f64,
    /// Federal and state operating assistance (`operating_assistance_share` x operating cost).
    pub operating_assistance: f64,
    /// Local transit sales tax collected in the served area.
    pub local_tax: f64,
    /// Capital spent on building or extending lines this month (gross).
    pub capital_spent: f64,
    /// Federal capital grants received against that spend.
    pub federal_capital_grants: f64,
    /// Change in cash: revenue + assistance + tax + grants - operating cost - capital.
    pub net: f64,
}

impl MonthStatement {
    fn new(month: u64) -> Self {
        MonthStatement {
            month,
            ..Default::default()
        }
    }
}

/// The transit authority's books.
#[derive(Debug, Clone, Default, Serialize)]
pub struct Ledger {
    /// Cash on hand, dollars. May go negative until bonds (F26) exist.
    pub cash: f64,
    /// The month in progress: capital charges so far, operating lines filled at settlement.
    pub current: MonthStatement,
    /// Settled months, oldest first.
    pub history: Vec<MonthStatement>,
    /// Residents within walking range of a stop (population x coverage share), at last settlement.
    pub served_population: f64,
}

impl Ledger {
    /// A ledger starting with `cash` in month `month`.
    pub fn new(cash: f64, month: u64) -> Self {
        Ledger {
            cash,
            current: MonthStatement::new(month),
            ..Default::default()
        }
    }

    /// Pay for construction now: `spend` gross, `grant` back from the federal government.
    pub fn charge_capital(&mut self, spend: f64, grant: f64) {
        self.current.capital_spent += spend;
        self.current.federal_capital_grants += grant;
        self.cash += grant - spend;
    }

    /// Close the current month using today's daily line results and the
    /// served population, then open the next month.
    pub fn settle(&mut self, lines: &[LineResult], served_population: f64, p: &Params) {
        const DAYS: f64 = crate::clock::DAYS_PER_MONTH as f64;
        let m = &mut self.current;
        m.fare_revenue = DAYS * lines.iter().map(|l| l.fare_revenue_per_day).sum::<f64>();
        m.operating_cost = DAYS * lines.iter().map(|l| l.operating_cost_per_day).sum::<f64>();
        m.operating_assistance = m.operating_cost * p.operating_assistance_share;
        m.local_tax =
            served_population * p.taxable_sales_per_capita * p.local_sales_tax_rate / 12.0;
        let operating = m.fare_revenue + m.operating_assistance + m.local_tax - m.operating_cost;
        self.cash += operating;
        m.net = operating + m.federal_capital_grants - m.capital_spent;
        self.served_population = served_population;
        let next = MonthStatement::new(m.month + 1);
        self.history
            .push(std::mem::replace(&mut self.current, next));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(boardings: f64, cost: f64, fares: f64) -> LineResult {
        LineResult {
            id: 0,
            name: String::new(),
            mode: crate::Mode::Bus,
            length_km: 10.0,
            run_min: 20.0,
            daily_boardings: boardings,
            boardings_by_stop: vec![],
            passenger_km: 0.0,
            max_segment_load: 0.0,
            peak_hour_load: 0.0,
            peak_hour_capacity: 0.0,
            vehicles_required: 4.0,
            revenue_hours_per_day: 0.0,
            operating_cost_per_day: cost,
            fare_revenue_per_day: fares,
            farebox_recovery: 0.0,
            segment_loads: vec![],
        }
    }

    #[test]
    fn grant_gate_and_settlement() {
        let p = Params::default();
        let cap = p.federal_max_capital_per_daily_rider;
        assert_eq!(
            federal_grant(100.0, cap * 10.0, 10.0, &p),
            100.0 * p.federal_capital_share
        );
        assert_eq!(federal_grant(100.0, cap * 10.0, 9.0, &p), 0.0);

        let mut l = Ledger::new(0.0, 0);
        l.charge_capital(1000.0, 500.0);
        assert_eq!(l.cash, -500.0);
        l.settle(&[line(100.0, 10.0, 1.0)], 1000.0, &p);
        let m = &l.history[0];
        assert_eq!(m.operating_cost, 300.0);
        assert_eq!(m.fare_revenue, 30.0);
        let tax = 1000.0 * p.taxable_sales_per_capita * p.local_sales_tax_rate / 12.0;
        assert!((m.local_tax - tax).abs() < 1e-9);
        assert!((l.cash - (-500.0 + m.net + 500.0)).abs() < 1e-9);
        assert_eq!(l.current.month, 1);
        assert_eq!(l.current.capital_spent, 0.0);
    }
}
