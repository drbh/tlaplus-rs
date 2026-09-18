use crate::{Error, behavior::Atom};
use std::collections::HashMap;
use tla_ir::{Expr, ExprId, Level};
use tla_model::{BoundModel, Value};
use tla_modules::temporal::{Template, Temporal};
use tla_props::Formula;

/// Temporal formulas over atoms that are IR predicates of the configured model.
pub struct Instantiated {
    pub assumptions: Vec<Formula>,
    pub properties: Vec<(String, Formula)>,
    pub atoms: Vec<Atom>,
}

/// Expands quantifiers over their (constant) domains and applies each predicate
/// definition to the values bound around it.
pub fn instantiate(model: &mut BoundModel, temporal: &Temporal) -> Result<Instantiated, Error> {
    let mut builder = Builder {
        model,
        atoms: Vec::new(),
        index: HashMap::new(),
    };
    let assumptions = temporal
        .assumptions
        .iter()
        .map(|t| builder.formula(t, &mut Vec::new()))
        .collect::<Result<_, _>>()?;
    let properties = temporal
        .properties
        .iter()
        .map(|(name, t)| Ok((name.clone(), builder.formula(t, &mut Vec::new())?)))
        .collect::<Result<_, Error>>()?;
    Ok(Instantiated {
        assumptions,
        properties,
        atoms: builder.atoms,
    })
}

struct Builder<'a> {
    model: &'a mut BoundModel,
    atoms: Vec<Atom>,
    index: HashMap<(String, bool, Vec<Value>), usize>,
}

impl Builder<'_> {
    fn formula(&mut self, template: &Template, values: &mut Vec<Value>) -> Result<Formula, Error> {
        Ok(match template {
            Template::Atom(name) => Formula::atom(self.atom(name, false, values)?),
            Template::Not(t) => self.formula(t, values)?.negate(),
            Template::And(a, b) => self.formula(a, values)?.and(self.formula(b, values)?),
            Template::Or(a, b) => self.formula(a, values)?.or(self.formula(b, values)?),
            Template::Always(t) => self.formula(t, values)?.always(),
            Template::Eventually(t) => self.formula(t, values)?.eventually(),
            Template::Fairness {
                strong,
                action,
                enabled,
            } => {
                let enabled = self.atom(enabled, false, values)?;
                let taken = self.atom(action, false, values)?;
                if *strong {
                    Formula::strong_fairness(enabled, taken)
                } else {
                    Formula::weak_fairness(enabled, taken)
                }
            }
            Template::Quantified {
                universal,
                domain,
                body,
            } => {
                let domain = self.apply(domain, values)?;
                let domain = self
                    .model
                    .evaluate(domain, &[], &[])
                    .map_err(|e| tla_model::Error(format!("temporal quantifier domain: {e}")))?;
                let mut result = if *universal {
                    Formula::True
                } else {
                    Formula::False
                };
                for value in domain.elements().map_err(tla_model::Error::from)? {
                    values.push(value.clone());
                    let formula = self.formula(body, values);
                    values.pop();
                    result = if *universal {
                        result.and(formula?)
                    } else {
                        result.or(formula?)
                    };
                }
                result
            }
        })
    }

    fn atom(&mut self, name: &str, enabled: bool, values: &[Value]) -> Result<usize, Error> {
        let key = (name.to_owned(), enabled, values.to_vec());
        if let Some(&id) = self.index.get(&key) {
            return Ok(id);
        }
        let predicate = self.apply(name, values)?;
        self.atoms.push(Atom { predicate, enabled });
        self.index.insert(key, self.atoms.len() - 1);
        Ok(self.atoms.len() - 1)
    }

    /// A call of the named definition on literal arguments.
    fn apply(&mut self, name: &str, values: &[Value]) -> Result<ExprId, Error> {
        let program = &mut self.model.program;
        let operator = program
            .operators
            .iter()
            .position(|op| op.name == name)
            .ok_or_else(|| Error::MissingPredicate(name.to_owned()))?;
        let level = program.nodes[program.operators[operator].body].level;
        let args = values
            .iter()
            .map(|v| program.push(Expr::Literal(tla_model::literal(v)), Level::Constant))
            .collect();
        Ok(program.push(Expr::Call { operator, args }, level))
    }
}
