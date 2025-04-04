#[cfg(not(test))]
use chrono::Local;

use chrono::NaiveDate;
use nom::IResult;
use nom::Parser;
use nom::branch::alt;
use nom::bytes::complete::{tag, take_while};
use nom::character::complete::{alpha1, digit1, multispace0, multispace1, space1};
use nom::combinator::{map, map_res, opt};
use nom::multi::many_m_n;
use nom::sequence::{pair, preceded};
use std::ops::RangeInclusive;
use std::str::FromStr;

#[cfg(not(test))]
fn today() -> NaiveDate {
    Local::now().date_naive()
}

#[cfg(test)]
fn today() -> NaiveDate {
    NaiveDate::from_ymd_opt(2025, 5, 4).unwrap()
}

fn parse_date(input: &str) -> IResult<&str, NaiveDate> {
    map_res(
        take_while(|x: char| x.is_ascii_digit() || x == '-'),
        |x: &str| NaiveDate::parse_from_str(x, "%Y-%m-%d"),
    )
    .parse(input)
}

fn parse_today(input: &str) -> IResult<&str, NaiveDate> {
    map(tag("today").or(tag("now")), |_| today()).parse(input)
}

fn parse_yesterday(input: &str) -> IResult<&str, NaiveDate> {
    map(tag("yesterday"), |_| today().pred_opt().unwrap()).parse(input)
}

#[derive(Debug, PartialEq, Eq)]
pub enum TimeOffsetUnit {
    Days,
    Weeks,
    Months,
    Years,
}

#[derive(Debug, PartialEq, Eq)]
pub struct TimeOffset {
    pub amount: u32,
    pub unit: TimeOffsetUnit,
}

impl TimeOffset {
    fn into_date(self) -> NaiveDate {
        let today_ = today();
        match self.unit {
            TimeOffsetUnit::Days => today_ - chrono::TimeDelta::days(self.amount as i64),
            TimeOffsetUnit::Weeks => today_ - chrono::TimeDelta::weeks(self.amount as i64),
            TimeOffsetUnit::Months => today_
                .checked_sub_months(chrono::Months::new(self.amount))
                .unwrap(),
            TimeOffsetUnit::Years => today_
                .checked_sub_months(chrono::Months::new(self.amount * 12))
                .unwrap(),
        }
    }
}

fn parse_number(input: &str) -> IResult<&str, u32> {
    map(digit1, |s: &str| u32::from_str(s).unwrap()).parse(input)
}

fn parse_unit(input: &str) -> IResult<&str, TimeOffsetUnit> {
    alt((
        map(tag("days").or(tag("day")), |_| TimeOffsetUnit::Days),
        map(tag("weeks").or(tag("week")), |_| TimeOffsetUnit::Weeks),
        map(tag("months").or(tag("month")), |_| TimeOffsetUnit::Months),
        map(tag("years").or(tag("year")), |_| TimeOffsetUnit::Years),
    ))
    .parse(input)
}

fn parse_suffix(input: &str) -> IResult<&str, ()> {
    let suffix = alt((tag("ago"), tag("before")));
    map(preceded(space1, suffix), |_| ()).parse(input)
}

pub fn parse_offset(input: &str) -> IResult<&str, NaiveDate> {
    let with_number = map(
        (parse_number, space1, parse_unit, opt(parse_suffix)),
        |(amount, _, unit, _)| TimeOffset { amount, unit },
    );

    let without_number = map(pair(parse_unit, parse_suffix), |(unit, _)| TimeOffset {
        amount: 1,
        unit,
    });

    map(alt((with_number, without_number)), TimeOffset::into_date).parse(input)
}

fn parse_date_or_label(input: &str) -> IResult<&str, NaiveDate> {
    alt((parse_today, parse_yesterday, parse_date, parse_offset)).parse(input)
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum Attr {
    Updated,
    Created,
}

impl FromStr for Attr {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "updated" => Ok(Self::Updated),
            "created" => Ok(Self::Created),
            _ => Err(()),
        }
    }
}

fn parse_attr(input: &str) -> IResult<&str, Attr> {
    map_res(alt((tag("updated"), tag("created"))), Attr::from_str).parse(input)
}

#[derive(Debug, Copy, Clone)]
enum Boundary {
    From(NaiveDate),
    To(NaiveDate),
}

impl TryFrom<(&str, NaiveDate)> for Boundary {
    type Error = ();

    fn try_from((tag, date): (&str, NaiveDate)) -> Result<Self, Self::Error> {
        match tag {
            "from" | "after" => Ok(Self::From(date)),
            "to" | "until" | "till" | "before" => Ok(Self::To(date)),
            _ => Err(()),
        }
    }
}

fn parse_boundary(input: &str) -> IResult<&str, Boundary> {
    map_res(
        (alpha1, multispace1, parse_date_or_label),
        |(tag, _, date)| Boundary::try_from((tag, date)),
    )
    .parse(input)
}

fn parse_range(input: &str) -> IResult<&str, (Option<NaiveDate>, Option<NaiveDate>)> {
    map_res(
        many_m_n(1, 2, preceded(multispace0, parse_boundary)),
        |x| match x.as_slice() {
            [Boundary::From(dt)] => Ok((Some(*dt), None)),
            [Boundary::To(dt)] => Ok((None, Some(*dt))),
            [Boundary::From(lower), Boundary::To(upper)] => Ok((Some(*lower), Some(*upper))),
            [Boundary::To(upper), Boundary::From(lower)] => Ok((Some(*lower), Some(*upper))),
            _ => Err(()),
        },
    )
    .parse(input)
}

pub fn parse_attr_range(input: &str) -> IResult<&str, (Attr, RangeInclusive<NaiveDate>)> {
    map(
        (
            preceded(multispace0, parse_attr),
            preceded(multispace1, parse_range),
        ),
        |(attr, (lower, upper))| {
            (
                attr,
                lower.unwrap_or(NaiveDate::MIN)..=upper.unwrap_or(NaiveDate::MAX),
            )
        },
    )
    .parse(input)
}

#[test]
fn test_ok_parse_attr_range() {
    let cases = [
        (
            "updated after 1 year ago before now",
            (Attr::Updated, ("2024-05-04", "2025-05-04")),
        ),
        (
            "created  before yesterday",
            (Attr::Created, ("MIN", "2025-05-03")),
        ),
        (
            "created  after 3 weeks ago",
            (Attr::Created, ("2025-04-13", "MAX")),
        ),
        (
            "created from 2023-06-07 to 2023-07-08",
            (Attr::Created, ("2023-06-07", "2023-07-08")),
        ),
        (
            "created after 3 months before before yesterday",
            (Attr::Created, ("2025-02-04", "2025-05-03")),
        ),
    ];

    for (input, (expected_attr, (from, to))) in cases {
        let from_dt = match from {
            "MIN" => NaiveDate::MIN,
            v => NaiveDate::from_str(v).unwrap(),
        };
        let to_dt = match to {
            "MAX" => NaiveDate::MAX,
            v => NaiveDate::from_str(v).unwrap(),
        };
        let expected_range = from_dt..=to_dt;

        let (tail, (attr, range)) = parse_attr_range(input).unwrap();
        assert!(tail.is_empty());
        assert_eq!(attr, expected_attr);
        assert_eq!(range, expected_range);
    }
}
