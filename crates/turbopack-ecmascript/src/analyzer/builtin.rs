use std::{mem::take, sync::Arc};

use super::{ConstantNumber, ConstantValue, JsValue, LogicalOperator, ObjectPart};
use crate::analyzer::FreeVarKind;

/// Replaces some builtin values with their resulting values. Called early
/// without lazy nested values.
pub fn early_replace_builtin(value: &mut JsValue) -> bool {
    match value {
        JsValue::Call(_, box ref mut callee, _) => match callee {
            JsValue::Unknown(_, _) => {
                value.make_unknown("unknown callee");
                true
            }
            JsValue::Constant(_)
            | JsValue::Url(_)
            | JsValue::WellKnownObject(_)
            | JsValue::Array(_, _)
            | JsValue::Object(_, _)
            | JsValue::Alternatives(_, _)
            | JsValue::Concat(_, _)
            | JsValue::Add(_, _)
            | JsValue::Not(_, _) => {
                value.make_unknown("non-function callee");
                true
            }
            _ => false,
        },
        JsValue::MemberCall(_, box ref mut obj, box ref mut prop, _) => match obj {
            JsValue::Unknown(_, _) => {
                value.make_unknown("unknown callee object");
                true
            }
            _ => match prop {
                JsValue::Unknown(_, _) => {
                    value.make_unknown("unknown calee property");
                    true
                }
                _ => false,
            },
        },
        JsValue::Member(_, box JsValue::Unknown(_, _), _) => {
            value.make_unknown("unknown object");
            true
        }
        _ => false,
    }
}

pub fn replace_builtin(value: &mut JsValue) -> bool {
    match value {
        // Accessing a property on something can be handled in some cases
        JsValue::Member(_, box ref mut obj, ref mut prop) => match obj {
            JsValue::Alternatives(_, alts) => {
                *value = JsValue::alternatives(
                    take(alts)
                        .into_iter()
                        .map(|alt| JsValue::member(box alt, prop.clone()))
                        .collect(),
                );
                true
            }
            JsValue::Array(_, array) => {
                fn items_to_alternatives(items: &mut Vec<JsValue>, prop: &mut JsValue) -> JsValue {
                    items.push(JsValue::Unknown(
                        Some(Arc::new(JsValue::member(
                            box JsValue::array(Vec::new()),
                            box take(prop),
                        ))),
                        "unknown array prototype methods or values",
                    ));
                    JsValue::alternatives(take(items))
                }
                match &mut **prop {
                    JsValue::Constant(ConstantValue::Num(ConstantNumber(num))) => {
                        let index: usize = *num as usize;
                        if index as f64 == *num && index < array.len() {
                            *value = array.swap_remove(index);
                            true
                        } else {
                            *value = JsValue::Unknown(
                                Some(Arc::new(JsValue::member(box take(obj), box take(prop)))),
                                "invalid index",
                            );
                            true
                        }
                    }
                    JsValue::Constant(_) => {
                        value.make_unknown("non-num constant property on array");
                        true
                    }
                    JsValue::Alternatives(_, alts) => {
                        *value = JsValue::alternatives(
                            take(alts)
                                .into_iter()
                                .map(|alt| JsValue::member(box obj.clone(), box alt))
                                .collect(),
                        );
                        true
                    }
                    _ => {
                        *value = items_to_alternatives(array, prop);
                        true
                    }
                }
            }
            JsValue::Object(_, parts) => {
                fn parts_to_alternatives(
                    parts: &mut Vec<ObjectPart>,
                    prop: &mut Box<JsValue>,
                ) -> JsValue {
                    let mut values = Vec::new();
                    for part in parts {
                        match part {
                            ObjectPart::KeyValue(_, value) => {
                                values.push(take(value));
                            }
                            ObjectPart::Spread(_) => {
                                values.push(JsValue::Unknown(
                                    Some(Arc::new(JsValue::member(
                                        box JsValue::object(vec![take(part)]),
                                        prop.clone(),
                                    ))),
                                    "spreaded object",
                                ));
                            }
                        }
                    }
                    values.push(JsValue::Unknown(
                        Some(Arc::new(JsValue::member(
                            box JsValue::object(Vec::new()),
                            box take(prop),
                        ))),
                        "unknown object prototype methods or values",
                    ));
                    JsValue::alternatives(values)
                }
                match &mut **prop {
                    JsValue::Constant(_) => {
                        for part in parts.iter_mut().rev() {
                            match part {
                                ObjectPart::KeyValue(key, val) => {
                                    if key == &**prop {
                                        *value = take(val);
                                        return true;
                                    }
                                }
                                ObjectPart::Spread(_) => {
                                    value.make_unknown("spreaded object");
                                    return true;
                                }
                            }
                        }
                        *value = JsValue::FreeVar(FreeVarKind::Other("undefined".into()));
                        true
                    }
                    JsValue::Alternatives(_, alts) => {
                        *value = JsValue::alternatives(
                            take(alts)
                                .into_iter()
                                .map(|alt| JsValue::member(box obj.clone(), box alt))
                                .collect(),
                        );
                        true
                    }
                    _ => {
                        *value = parts_to_alternatives(parts, prop);
                        true
                    }
                }
            }
            _ => false,
        },
        JsValue::MemberCall(_, box ref mut obj, box ref mut prop, ref mut args) => {
            match obj {
                JsValue::Array(_, items) => {
                    if let Some(str) = prop.as_str() {
                        match str {
                            "concat" => {
                                if args.iter().all(|arg| {
                                    matches!(
                                        arg,
                                        JsValue::Array(..)
                                            | JsValue::Constant(_)
                                            | JsValue::Url(_)
                                            | JsValue::Concat(..)
                                            | JsValue::Add(..)
                                            | JsValue::WellKnownObject(_)
                                            | JsValue::WellKnownFunction(_)
                                            | JsValue::Function(..)
                                    )
                                }) {
                                    for arg in args {
                                        match arg {
                                            JsValue::Array(_, inner) => {
                                                items.extend(take(inner));
                                            }
                                            JsValue::Constant(_)
                                            | JsValue::Url(_)
                                            | JsValue::Concat(..)
                                            | JsValue::Add(..)
                                            | JsValue::WellKnownObject(_)
                                            | JsValue::WellKnownFunction(_)
                                            | JsValue::Function(..) => {
                                                items.push(take(arg));
                                            }
                                            _ => {
                                                unreachable!();
                                            }
                                        }
                                    }
                                    obj.update_total_nodes();
                                    *value = take(obj);
                                    return true;
                                }
                            }
                            "map" => {
                                if let Some(func) = args.get(0) {
                                    *value = JsValue::array(
                                        take(items)
                                            .into_iter()
                                            .enumerate()
                                            .map(|(i, item)| {
                                                JsValue::call(
                                                    box func.clone(),
                                                    vec![
                                                        item,
                                                        JsValue::Constant(ConstantValue::Num(
                                                            ConstantNumber(i as f64),
                                                        )),
                                                    ],
                                                )
                                            })
                                            .collect(),
                                    );
                                    return true;
                                }
                            }
                            _ => {}
                        }
                    }
                }
                JsValue::Alternatives(_, alts) => {
                    *value = JsValue::alternatives(
                        take(alts)
                            .into_iter()
                            .map(|alt| {
                                JsValue::member_call(box alt, box prop.clone(), args.clone())
                            })
                            .collect(),
                    );
                    return true;
                }
                _ => {}
            }
            *value = JsValue::call(
                box JsValue::member(box take(obj), box take(prop)),
                take(args),
            );
            true
        }
        // Handle calls when the callee is a function
        JsValue::Call(_, box JsValue::Alternatives(_, alts), ref mut args) => {
            *value = JsValue::alternatives(
                take(alts)
                    .into_iter()
                    .map(|alt| JsValue::call(box alt, args.clone()))
                    .collect(),
            );
            true
        }
        // Handle spread in object literals
        JsValue::Object(_, parts) => {
            if parts
                .iter()
                .any(|part| matches!(part, ObjectPart::Spread(JsValue::Object(..))))
            {
                let old_parts = take(parts);
                for part in old_parts {
                    if let ObjectPart::Spread(JsValue::Object(_, inner_parts)) = part {
                        parts.extend(inner_parts);
                    } else {
                        parts.push(part);
                    }
                }
                value.update_total_nodes();
                true
            } else {
                false
            }
        }
        // Reduce logical expressions to their final value(s)
        JsValue::Logical(_, op, ref mut parts) => {
            let len = parts.len();
            for (i, part) in take(parts).into_iter().enumerate() {
                // The last part is never skipped.
                if i == len - 1 {
                    parts.push(part);
                    break;
                }
                // We might know at compile-time if a part is skipped or the final value.
                let skip_part = match op {
                    LogicalOperator::And => part.is_truthy(),
                    LogicalOperator::Or => part.is_falsy(),
                    LogicalOperator::NullishCoalescing => part.is_nullish(),
                };
                match skip_part {
                    Some(true) => {
                        // We known this part is skipped, so we can remove it.
                        continue;
                    }
                    Some(false) => {
                        // We known this part is the final value, so we can remove the rest.
                        parts.push(part);
                        break;
                    }
                    None => {
                        // We don't know if this part is skipped or the final value, so we keep it.
                        parts.push(part);
                        continue;
                    }
                }
            }
            // If we reduced the expression to a single value, we can replace it.
            if parts.len() == 1 {
                *value = parts.pop().unwrap();
                true
            } else {
                // If not, we known that it will be one of the remaining values.
                *value = JsValue::alternatives(take(parts));
                true
            }
        }
        // Evaluate not when the inner value is truthy or falsy
        JsValue::Not(_, ref inner) => match inner.is_truthy() {
            Some(true) => {
                *value = JsValue::Constant(ConstantValue::False);
                true
            }
            Some(false) => {
                *value = JsValue::Constant(ConstantValue::True);
                true
            }
            None => false,
        },
        _ => false,
    }
}
