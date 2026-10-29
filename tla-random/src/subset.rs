use crate::Random;
use num_bigint::BigInt;
use std::sync::OnceLock;
use tlaplus_value_expr::Expr;
use tlaplus_values::{Error, Value};

fn factors(mut value: i32) -> Vec<i32> {
    let mut result = Vec::new();
    let mut divisor = 2;
    while i64::from(divisor) * i64::from(divisor) <= i64::from(value) {
        while value % divisor == 0 {
            result.push(divisor);
            value /= divisor;
        }
        divisor += 1;
    }
    if value > 1 {
        result.push(value);
    }
    result
}

fn multipliers(size: i32) -> Result<(i32, i32), Error> {
    let mut modulus = size.max(9);
    loop {
        let mut divisors = factors(modulus);
        let count = divisors.len();
        divisors.dedup();
        if modulus % 4 != 0 && divisors.len() != count {
            return Ok((modulus, divisors.iter().product::<i32>() + 1));
        }
        modulus = modulus.checked_add(1).ok_or(Error::Overflow)?;
    }
}

fn primes() -> &'static [i32] {
    static PRIMES: OnceLock<Vec<i32>> = OnceLock::new();
    PRIMES.get_or_init(|| {
        (1_277_011..=1_295_953)
            .filter(|value| factors(*value).len() == 1)
            .collect()
    })
}

impl Random {
    pub fn subset(&mut self, count: i32, set: &Expr, limit: usize) -> Result<Value, Error> {
        if count < 0 {
            return Err(Error::Type("nonnegative subset size"));
        }
        if count as usize > limit {
            return Err(Error::Limit);
        }
        if matches!(
            set,
            Expr::Product(_) | Expr::Records(_) | Expr::Functions { .. }
        ) {
            return self.product_subset(count, set, limit);
        }
        let (size, values) = match set {
            Expr::Interval(low, high) => (
                i32::try_from((i64::from(*high) - i64::from(*low) + 1).max(0))
                    .map_err(|_| Error::Overflow)?,
                None,
            ),
            _ => {
                let values = set.sorted(limit)?;
                (
                    i32::try_from(values.len()).map_err(|_| Error::Overflow)?,
                    Some(values),
                )
            }
        };
        if size == 0 {
            return Value::set(vec![]);
        }
        Value::set(
            self.indices(count, size)?
                .into_iter()
                .map(|index| {
                    Ok(match (&values, set) {
                        (None, Expr::Interval(low, _)) => Value::Int(low.wrapping_add(index)),
                        (Some(values), _) => values[index as usize].normalize(limit)?,
                        _ => unreachable!(),
                    })
                })
                .collect::<Result<_, Error>>()?,
        )
    }

    fn indices(&mut self, count: i32, size: i32) -> Result<Vec<i32>, Error> {
        let (modulus, multiplier) = multipliers(size)?;
        let mut index = self.next_bounded(size)?;
        let primes = primes();
        let increment = primes[(self.next_double() * primes.len() as f64) as usize];
        Ok((0..count)
            .map(|_| {
                loop {
                    index = ((i64::from(multiplier) * i64::from(index) + i64::from(increment))
                        % i64::from(modulus)) as i32;
                    if index < size {
                        break index;
                    }
                }
            })
            .collect())
    }

    fn product_subset(&mut self, count: i32, set: &Expr, limit: usize) -> Result<Value, Error> {
        let (keys, domains) = match set {
            Expr::Product(domains) => (None, domains.clone()),
            Expr::Records(fields) => {
                let mut fields: Vec<_> = fields.iter().collect();
                fields.sort_by_key(|(name, _)| name.ordinal);
                (
                    Some(
                        fields
                            .iter()
                            .map(|(name, _)| Value::String(name.clone()))
                            .collect::<Vec<_>>(),
                    ),
                    fields.iter().map(|(_, domain)| domain.clone()).collect(),
                )
            }
            Expr::Functions { domain, range } => {
                let keys = domain
                    .sorted(limit)?
                    .iter()
                    .map(|v| v.normalize(limit))
                    .collect::<Result<Vec<_>, _>>()?;
                let domains = vec![(**range).clone(); keys.len()];
                (Some(keys), domains)
            }
            _ => unreachable!(),
        };
        if domains.iter().any(|domain| domain.empty().unwrap_or(false)) {
            return Value::set(vec![]);
        }
        let domains = domains
            .iter()
            .map(|domain| domain.sorted(limit))
            .collect::<Result<Vec<_>, _>>()?;
        let size: BigInt = domains
            .iter()
            .map(|domain| BigInt::from(domain.len()))
            .product();
        let indices = if size <= BigInt::from(i32::MAX) {
            self.indices(count, i32::try_from(&size).map_err(|_| Error::Overflow)?)?
                .into_iter()
                .map(BigInt::from)
                .collect::<Vec<_>>()
        } else {
            let offset = BigInt::from(self.next_long().wrapping_abs());
            (0..count)
                .map(|i| {
                    let index = (BigInt::from(i64::MAX - 24) * i + &offset) % &size;
                    if index < BigInt::from(0) {
                        index + &size
                    } else {
                        index
                    }
                })
                .collect()
        };
        Value::set(
            indices
                .into_iter()
                .map(|mut index| {
                    let mut values = Vec::with_capacity(domains.len());
                    for domain in domains.iter().rev() {
                        let digit =
                            usize::try_from(&index % domain.len()).map_err(|_| Error::Overflow)?;
                        values.push(domain[digit].normalize(limit)?);
                        index /= domain.len();
                    }
                    values.reverse();
                    match &keys {
                        Some(keys) => Value::function(keys.iter().cloned().zip(values).collect()),
                        None => Ok(Value::tuple(values)),
                    }
                })
                .collect::<Result<_, Error>>()?,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tlc_prime_range_and_multiplier_boundaries() {
        assert_eq!(primes().len(), 1341);
        assert_eq!(multipliers(1), Ok((9, 4)));
        assert_eq!(multipliers(10), Ok((18, 7)));
        assert_eq!(multipliers(i32::MAX), Err(Error::Overflow));
        let mut random = Random::new(42);
        assert_eq!(random.next_bounded(10), Ok(0));
        assert_eq!(random.next_bounded(10), Ok(3));
        assert_eq!(Random::new(0).next_long(), -4_962_768_465_676_381_896);
    }
}
