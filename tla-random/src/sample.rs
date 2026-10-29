use crate::Random;
use tlaplus_value_expr::Expr;
use tlaplus_values::{Error, Value};

impl Random {
    /// Draws for `RandomElement`, `RandomSubset` or `RandomSetOfSubsets`, selected
    /// by the number of already evaluated count operands.
    pub fn sample(&mut self, counts: &[Value], set: &Expr, limit: usize) -> Result<Value, Error> {
        match counts {
            [] => self.element(set, limit),
            [count] => self.subset(count.integer()?, set, limit),
            [picks, n] => {
                let set = set.normalize(limit)?;
                self.set_of_subsets(picks.integer()?, n.integer()?, set.elements()?)
            }
            _ => Err(Error::Type("random sampling operands")),
        }
    }

    pub fn set_of_subsets(
        &mut self,
        picks: i32,
        n: i32,
        elements: &[Value],
    ) -> Result<Value, Error> {
        let size = i32::try_from(elements.len()).map_err(|_| Error::Overflow)?;
        // An empty set yields probability 0/0, which TLC rejects as out of range.
        if picks < 0 || n < 0 || n > size || size == 0 || (size < 63 && picks as u64 > 1u64 << size)
        {
            return Err(Error::Type("RandomSetOfSubsets arguments"));
        }
        let probability = f64::from(n) / f64::from(size);
        let mut subsets = Vec::with_capacity(picks as usize);
        for _ in 0..picks {
            subsets.push(Value::set(
                elements
                    .iter()
                    .filter(|_| self.next_double() < probability)
                    .cloned()
                    .collect(),
            )?);
        }
        Value::set(subsets)
    }

    pub fn element(&mut self, set: &Expr, limit: usize) -> Result<Value, Error> {
        normalize_components(set, limit)?;
        self.draw(set, limit)
    }

    fn draw(&mut self, set: &Expr, limit: usize) -> Result<Value, Error> {
        match set {
            Expr::Interval(low, high) => {
                let size = (i64::from(*high) - i64::from(*low) + 1).max(0);
                let size = i32::try_from(size).map_err(|_| Error::Overflow)?;
                let offset = (self.next_double() * f64::from(size)) as i64;
                if size == 0 {
                    return Err(Error::Undefined);
                }
                Ok(Value::Int((i64::from(*low) + offset) as i32))
            }
            Expr::Functions { domain, range } => Value::function(
                domain
                    .sorted(limit)?
                    .iter()
                    .map(|key| Ok((key.normalize(limit)?, self.draw(range, limit)?)))
                    .collect::<Result<_, Error>>()?,
            ),
            Expr::Product(sets) => Ok(Value::tuple(
                sets.iter()
                    .map(|set| self.draw(set, limit))
                    .collect::<Result<_, _>>()?,
            )),
            Expr::Records(fields) => {
                let mut fields: Vec<_> = fields.iter().collect();
                fields.sort_by_key(|(name, _)| name.ordinal);
                Value::record(
                    fields
                        .into_iter()
                        .map(|(name, set)| Ok((name.clone(), self.draw(set, limit)?)))
                        .collect::<Result<_, Error>>()?,
                )
            }
            _ => {
                let values = set.sorted(limit)?;
                let index = (self.next_double() * values.len() as f64) as usize;
                values.get(index).ok_or(Error::Undefined)?.normalize(limit)
            }
        }
    }
}

fn normalize_components(set: &Expr, limit: usize) -> Result<(), Error> {
    match set {
        Expr::Set(_) => {
            set.sorted(limit)?;
        }
        Expr::Functions { domain, range } => {
            normalize_components(domain, limit)?;
            normalize_components(range, limit)?;
        }
        Expr::Product(sets) => {
            for set in sets {
                normalize_components(set, limit)?;
            }
        }
        Expr::Records(fields) => {
            let mut fields: Vec<_> = fields.iter().collect();
            fields.sort_by_key(|(name, _)| name.ordinal);
            for (_, set) in fields {
                normalize_components(set, limit)?;
            }
        }
        Expr::Subsets { base, .. } => normalize_components(base, limit)?,
        _ => {}
    }
    Ok(())
}
