#[cfg(not(test))]
use chrono::Local;

use chrono::{Datelike, NaiveDate};
use nom::IResult;
use nom::Parser;
use nom::branch::alt;
use nom::bytes::complete::{tag, take_while1};
use nom::character::complete::{digit1, multispace0, multispace1, space0, space1};
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

fn alpha1_utf8(input: &str) -> IResult<&str, &str> {
    take_while1(|c: char| c.is_alphabetic()).parse(input)
}

fn iso_date(input: &str) -> IResult<&str, NaiveDate> {
    map_res(
        take_while1(|x: char| x.is_ascii_digit() || x == '-'),
        |x: &str| NaiveDate::parse_from_str(x, "%Y-%m-%d"),
    )
    .parse(input)
}

fn cis_date(input: &str) -> IResult<&str, NaiveDate> {
    map_res(
        take_while1(|x: char| x.is_ascii_digit() || x == '.'),
        |x: &str| {
            let parts = x
                .splitn(3, ".")
                .map(|x| x.parse::<u32>())
                .collect::<Result<Vec<_>, _>>()
                .map_err(|_| ())?;
            match parts.as_slice() {
                [d, m] => {
                    let cur_year = today().year();
                    Ok(NaiveDate::from_ymd_opt(cur_year, *m, *d).ok_or(())?)
                }
                [d, m, y] => Ok(NaiveDate::from_ymd_opt(*y as i32, *m, *d).ok_or(())?),
                _ => Err(()),
            }
        },
    )
    .parse(input)
}

fn parse_today(input: &str) -> IResult<&str, NaiveDate> {
    map(
        alt((tag("today"), tag("now"), tag("сегодня"), tag("сейчас"))),
        |_| today(),
    )
    .parse(input)
}

fn yesterday(input: &str) -> IResult<&str, NaiveDate> {
    map(tag("yesterday").or(tag("вчера")), |_| {
        today().pred_opt().unwrap()
    })
    .parse(input)
}

fn tdby(input: &str) -> IResult<&str, NaiveDate> {
    map(tag("позавчера"), |_| {
        today().pred_opt().unwrap().pred_opt().unwrap()
    })
    .parse(input)
}

#[derive(Debug, PartialEq, Eq)]
pub enum TimeUnit {
    Days,
    Weeks,
    Months,
    Years,
}

impl FromStr for TimeUnit {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "days" | "day" | "день" | "дней" | "дня" => Ok(Self::Days),
            "weeks" | "week" | "неделя" | "недели" | "недель" | "неделю" => {
                Ok(Self::Weeks)
            }
            "months" | "month" | "месяцев" | "месяца" | "месяц" => {
                Ok(Self::Months)
            }
            "years" | "year" | "года" | "год" | "лет" => Ok(Self::Years),
            _ => Err(()),
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct TimeOffset {
    pub amount: u32,
    pub unit: TimeUnit,
}

impl TimeOffset {
    fn into_date(self) -> NaiveDate {
        let today_ = today();
        match self.unit {
            TimeUnit::Days => today_ - chrono::TimeDelta::days(self.amount as i64),
            TimeUnit::Weeks => today_ - chrono::TimeDelta::weeks(self.amount as i64),
            TimeUnit::Months => today_
                .checked_sub_months(chrono::Months::new(self.amount))
                .unwrap(),
            TimeUnit::Years => today_
                .checked_sub_months(chrono::Months::new(self.amount * 12))
                .unwrap(),
        }
    }
}

fn number(input: &str) -> IResult<&str, u32> {
    map(digit1, |s: &str| u32::from_str(s).unwrap()).parse(input)
}

fn time_unit(input: &str) -> IResult<&str, TimeUnit> {
    map_res(alpha1_utf8, TimeUnit::from_str).parse(input)
}

fn time_suffix_en(input: &str) -> IResult<&str, ()> {
    let suffix = alt((tag("ago"), tag("before"), tag("назад")));
    map(preceded(space1, suffix), |_| ()).parse(input)
}

pub fn parse_offset(input: &str) -> IResult<&str, NaiveDate> {
    let with_number = map(
        (number, space1, time_unit, opt(time_suffix_en)),
        |(amount, _, unit, _)| TimeOffset { amount, unit },
    );

    let without_number = map(pair(time_unit, time_suffix_en), |(unit, _)| TimeOffset {
        amount: 1,
        unit,
    });

    map(alt((with_number, without_number)), TimeOffset::into_date).parse(input)
}

fn parse_date(input: &str) -> IResult<&str, NaiveDate> {
    alt((
        parse_today,
        yesterday,
        tdby,
        cis_date,
        iso_date,
        parse_offset,
    ))
    .parse(input)
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
            "updated" | "обновлено" => Ok(Self::Updated),
            "created" | "создано" => Ok(Self::Created),
            _ => Err(()),
        }
    }
}

fn attr(input: &str) -> IResult<&str, Attr> {
    map_res(alpha1_utf8, Attr::from_str).parse(input)
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
            "from" | "after" | "со" | "с" | "от" | "после" | "позже" => {
                Ok(Self::From(date))
            }
            "to" | "until" | "till" | "before" | "до" | "по" | "раньше" | "ранее" => {
                Ok(Self::To(date))
            }
            _ => Err(()),
        }
    }
}

fn boundary(input: &str) -> IResult<&str, Boundary> {
    map_res((alpha1_utf8, multispace1, parse_date), |(tag, _, date)| {
        Boundary::try_from((tag, date))
    })
    .parse(input)
}

fn date_range(input: &str) -> IResult<&str, (Option<NaiveDate>, Option<NaiveDate>)> {
    map_res(
        many_m_n(1, 2, preceded(multispace0, boundary)),
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

fn last_something_en(input: &str) -> IResult<&str, (Option<NaiveDate>, Option<NaiveDate>)> {
    map_res(
        (tag("last"), space0, opt(number), space0, time_unit),
        |(_, _, num, _, unit)| {
            let amount = num.unwrap_or(1);
            let start = TimeOffset { amount, unit }.into_date();
            Ok::<_, ()>((Some(start), Some(today())))
        },
    )
    .parse(input)
}

fn last_something_ru(input: &str) -> IResult<&str, (Option<NaiveDate>, Option<NaiveDate>)> {
    map_res(
        (
            opt(tag("за")),
            space0,
            opt(number),
            space0,
            alt((
                tag("прошлый"),
                tag("прошлых"),
                tag("прошлая"),
                tag("прошлую"),
                tag("последних"),
                tag("последний"),
                tag("последнюю"),
            )),
            space1,
            time_unit,
        ),
        |(_, _, num, _, _, _, unit)| {
            let amount = num.unwrap_or(1);
            let start = TimeOffset { amount, unit }.into_date();
            Ok::<_, ()>((Some(start), Some(today())))
        },
    )
    .parse(input)
}

fn one_day_range(input: &str) -> IResult<&str, (Option<NaiveDate>, Option<NaiveDate>)> {
    map(parse_date, |x| (Some(x), Some(x))).parse(input)
}

pub fn attr_and_range(input: &str) -> IResult<&str, (Attr, RangeInclusive<NaiveDate>)> {
    map(
        (
            preceded(multispace0, attr),
            preceded(
                multispace1,
                alt((
                    date_range,
                    one_day_range,
                    last_something_ru,
                    last_something_en,
                )),
            ),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ok_parse_attr_range() {
        let cases = [
            (
                "updated today",
                (Attr::Updated, ("2025-05-04", "2025-05-04")),
            ),
            (
                "updated last week",
                (Attr::Updated, ("2025-04-27", "2025-05-04")),
            ),
            (
                "updated last 7 days",
                (Attr::Updated, ("2025-04-27", "2025-05-04")),
            ),
            (
                "updated last 17 years",
                (Attr::Updated, ("2008-05-04", "2025-05-04")),
            ),
            (
                "обновлено за последнюю неделю",
                (Attr::Updated, ("2025-04-27", "2025-05-04")),
            ),
            (
                "обновлено за 3 последних недели",
                (Attr::Updated, ("2025-04-13", "2025-05-04")),
            ),
            (
                "обновлено за последний год",
                (Attr::Updated, ("2024-05-04", "2025-05-04")),
            ),
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
            (
                "обновлено со вчера до сегодня",
                (Attr::Updated, ("2025-05-03", "2025-05-04")),
            ),
            (
                "обновлено с 3 дня назад до позавчера",
                (Attr::Updated, ("2025-05-01", "2025-05-02")),
            ),
            (
                "обновлено с 02.03.2022 по 31.08",
                (Attr::Updated, ("2022-03-02", "2025-08-31")),
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

            let result = attr_and_range(input);
            assert!(result.is_ok(), "case '{input}' failed: {:?}", result.err());
            let (tail, (attr, range)) = result.unwrap();
            assert!(tail.is_empty(), "case '{input}' failed");
            assert_eq!(attr, expected_attr, "case '{input}' failed");
            assert_eq!(range, expected_range, "case '{input}' failed");
        }
    }

    #[test]
    fn test_parse_ru_date() {
        assert_eq!(
            cis_date("02.03").map(|(x, y)| (x, y.to_string())),
            Ok(("", "2025-03-02".to_string()))
        );

        assert_eq!(
            cis_date("31.03.2021").map(|(x, y)| (x, y.to_string())),
            Ok(("", "2021-03-31".to_string()))
        );
    }
}
