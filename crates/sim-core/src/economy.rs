//! Money: capital charges when lines are built, monthly settlement of fares,
//! operating cost, operating assistance and the local transit sales tax.
//!
//! Funding follows Collins's decision (D30): the network is paid for by
//! federal and state grants plus a local tax base, with revenue bonds to
//! spread capital over decades (F26). Twelve distressed months in a row
//! (debt service above operating surplus, or cash below zero) is bankruptcy,
//! the sandbox's one fail state (SIM-5).
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
    /// Cash raised by issuing bonds this month.
    pub bond_proceeds: f64,
    /// Bond payments this month (interest plus principal).
    pub debt_service: f64,
    /// Interest part of `debt_service`.
    pub interest: f64,
    /// Change in cash: revenue + assistance + tax + grants + bond proceeds
    /// - operating cost - capital - debt service.
    pub net: f64,
}

impl MonthStatement {
    /// Fares + operating assistance + local tax - operating cost: what the
    /// month leaves to pay debt with.
    pub fn operating_surplus(&self) -> f64 {
        self.fare_revenue + self.operating_assistance + self.local_tax - self.operating_cost
    }
}

/// A level-payment bond: the same payment every month until it is repaid.
#[derive(Debug, Clone, Serialize)]
pub struct Bond {
    /// Amount borrowed, dollars.
    pub principal: f64,
    /// Annual interest rate (0.0543 = 5.43%).
    pub annual_rate: f64,
    /// Monthly payments in all.
    pub term_months: u32,
    /// Game month it was issued; payments start the month after.
    pub issued_month: u64,
    /// Fixed monthly payment.
    pub payment: f64,
    /// Principal still owed.
    pub balance: f64,
    /// Payments still to make.
    pub months_left: u32,
    /// First month whose payment counts in the coverage test. Months before
    /// it are the construction period the bond funds (capitalized interest,
    /// SIM-7); the cash test applies throughout.
    pub coverage_from_month: u64,
}

impl Bond {
    /// A bond of `principal` at `annual_rate` repaid over `term_months`
    /// equal monthly payments (standard annuity formula).
    pub fn new(principal: f64, annual_rate: f64, term_months: u32, issued_month: u64) -> Self {
        let n = term_months.max(1);
        let r = annual_rate / 12.0;
        let payment = if r > 0.0 {
            principal * r / (1.0 - (1.0 + r).powi(-(n as i32)))
        } else {
            principal / n as f64
        };
        Bond {
            principal,
            annual_rate,
            term_months: n,
            issued_month,
            payment,
            balance: principal,
            months_left: n,
            coverage_from_month: issued_month + 1,
        }
    }

    /// Make one payment. Returns (payment, interest part).
    fn pay(&mut self) -> (f64, f64) {
        let interest = self.balance * self.annual_rate / 12.0;
        let payment = if self.months_left <= 1 {
            self.balance + interest
        } else {
            self.payment
        };
        self.balance = (self.balance + interest - payment).max(0.0);
        self.months_left -= 1;
        (payment, interest)
    }
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
    /// Bonds not yet repaid, oldest first.
    pub bonds: Vec<Bond>,
    /// Distressed months in a row so far (debt service above operating
    /// surplus, or cash below zero at month end). Resets on a healthy month.
    pub distressed_months: u32,
    /// Month the authority went bankrupt (`bankruptcy_months` distressed
    /// months in a row). The game is over from then on; the sim keeps running
    /// so the player can see why.
    pub bankrupt_month: Option<u64>,
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

    /// Borrow `principal` now at `annual_rate` over `term_months`. The cash
    /// arrives at once; payments start at the next month's settlement and
    /// enter the coverage test after `grace_months` more (SIM-7).
    pub fn issue_bond(
        &mut self,
        principal: f64,
        annual_rate: f64,
        term_months: u32,
        grace_months: u64,
    ) -> &Bond {
        let mut bond = Bond::new(principal, annual_rate, term_months, self.current.month);
        bond.coverage_from_month += grace_months;
        self.current.bond_proceeds += principal;
        self.cash += principal;
        self.bonds.push(bond);
        self.bonds.last().unwrap()
    }

    /// Monthly payments due on all outstanding bonds.
    pub fn monthly_debt_service(&self) -> f64 {
        self.bonds.iter().map(|b| b.payment).sum()
    }

    /// Principal still owed on all bonds.
    pub fn debt(&self) -> f64 {
        self.bonds.iter().map(|b| b.balance).sum()
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
        let operating = m.operating_surplus();
        let mut covered_service = 0.0;
        for b in self.bonds.iter_mut().filter(|b| b.issued_month < m.month) {
            let (payment, interest) = b.pay();
            m.debt_service += payment;
            m.interest += interest;
            if m.month >= b.coverage_from_month {
                covered_service += payment;
            }
        }
        self.bonds.retain(|b| b.months_left > 0);
        self.cash += operating - m.debt_service;
        m.net = operating + m.federal_capital_grants + m.bond_proceeds
            - m.capital_spent
            - m.debt_service;
        self.served_population = served_population;
        let coverage_fails = covered_service > operating;
        if coverage_fails || self.cash < 0.0 {
            self.distressed_months += 1;
        } else {
            self.distressed_months = 0;
        }
        if self.bankrupt_month.is_none() && self.distressed_months >= p.bankruptcy_months {
            self.bankrupt_month = Some(m.month);
        }
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
            segment_shapes: vec![],
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

    #[test]
    fn bond_amortizes_to_zero() {
        // $1M at 6% over 12 months: annuity payment $86,066.43.
        let mut b = Bond::new(1e6, 0.06, 12, 0);
        assert!((b.payment - 86_066.43).abs() < 0.01);
        let mut paid = 0.0;
        for _ in 0..12 {
            paid += b.pay().0;
        }
        assert!(b.balance.abs() < 1e-6 && b.months_left == 0);
        assert!((paid - 12.0 * 86_066.43).abs() < 1.0);
        assert_eq!(Bond::new(1200.0, 0.0, 12, 0).payment, 100.0);
    }

    #[test]
    fn bond_cash_flow_and_bankruptcy() {
        let p = Params {
            bankruptcy_months: 3,
            ..Params::default()
        };
        let mut l = Ledger::new(0.0, 0);
        let pay = l.issue_bond(1e6, 0.06, 12, 0).payment;
        assert_eq!(l.cash, 1e6);
        // Issue month: proceeds counted, no payment yet.
        l.settle(&[], 0.0, &p);
        assert_eq!(l.history[0].bond_proceeds, 1e6);
        assert_eq!(l.history[0].debt_service, 0.0);
        assert_eq!(l.distressed_months, 0);
        // No revenue at all: every paying month is distressed.
        for _ in 0..3 {
            l.settle(&[], 0.0, &p);
        }
        assert!((l.history[1].debt_service - pay).abs() < 1e-6);
        assert!((l.cash - (1e6 - 3.0 * pay)).abs() < 1e-3);
        assert_eq!(l.distressed_months, 3);
        assert_eq!(l.bankrupt_month, Some(3));
        assert!(l.debt() < 1e6);
    }

    #[test]
    fn grace_months_skip_the_coverage_test_not_the_cash_test() {
        let p = Params::default();
        let mut l = Ledger::new(0.0, 0);
        // Issued in month 0 with 2 grace months: payments in months 1 and 2
        // are outside the coverage test, month 3 is inside.
        assert_eq!(l.issue_bond(1e6, 0.06, 12, 2).coverage_from_month, 3);
        l.settle(&[], 0.0, &p);
        l.settle(&[], 0.0, &p);
        l.settle(&[], 0.0, &p);
        assert!(l.history[2].debt_service > 0.0);
        assert_eq!(l.distressed_months, 0, "grace: coverage not tested");
        l.settle(&[], 0.0, &p);
        assert_eq!(l.distressed_months, 1, "after grace: coverage fails");
        let mut l = Ledger::new(-1.0, 0);
        l.issue_bond(1.0, 0.06, 12, 6);
        l.cash = -1.0;
        l.settle(&[], 0.0, &p);
        assert_eq!(l.distressed_months, 1, "negative cash counts in grace");
    }
}
