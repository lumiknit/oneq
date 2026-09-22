use super::{
    JqError, Vm,
    code::{BuildValue, ContainerPlan},
};
use crate::{data::Value, jq::builtins::scalar};
use std::rc::Rc;

impl Vm {
    pub(super) fn build_container(
        &mut self,
        plan: &ContainerPlan,
        extend: bool,
    ) -> Result<(), JqError> {
        let left = if extend {
            std::mem::take(&mut self.input.value)
        } else {
            Value::Null
        };
        // Release evaluation-only aliases before COW, but retain all checkpoints.
        self.input = Value::Null.into();
        let count = match plan {
            ContainerPlan::Array(fields) => fields
                .iter()
                .filter(|f| matches!(f, BuildValue::Dynamic))
                .count(),
            ContainerPlan::Object(fields) => fields
                .iter()
                .filter(|(_, f)| matches!(f, BuildValue::Dynamic))
                .count(),
        };
        let mut inline = [const { Value::Null }; 8];
        let mut heap = Vec::new();
        let dynamic = if count <= inline.len() {
            &mut inline[..count]
        } else {
            heap.resize(count, Value::Null);
            heap.as_mut_slice()
        };
        for field in dynamic.iter_mut().rev() {
            *field = self
                .operands
                .pop()
                .ok_or_else(|| JqError::InvalidCode("container field missing".into()))?
                .value;
        }
        self.operands
            .pop()
            .ok_or_else(|| JqError::InvalidCode("container input missing".into()))?;
        let mut cursor = count;
        let mut build_value = |field: &BuildValue| match field {
            BuildValue::Constant(value) => value.clone(),
            BuildValue::Dynamic => {
                cursor -= 1;
                std::mem::take(&mut dynamic[cursor])
            }
        };
        let compatible = matches!(
            (&left, plan),
            (Value::Null, _)
                | (Value::Array(_), ContainerPlan::Array(_))
                | (Value::Object(_), ContainerPlan::Object(_))
        );
        let (mut result, fallback) = if compatible {
            (left, None)
        } else {
            (Value::Null, Some(left))
        };
        match plan {
            ContainerPlan::Array(fields) => {
                let mut array = match result {
                    Value::Array(a) => a,
                    _ => Rc::new(Vec::new()),
                };
                let values = Rc::make_mut(&mut array);
                let start = values.len();
                values.resize(start + fields.len(), Value::Null);
                for (index, field) in fields.iter().enumerate().rev() {
                    values[start + index] = build_value(field);
                }
                result = Value::Array(array);
            }
            ContainerPlan::Object(fields) => {
                let mut object = match result {
                    Value::Object(o) => o,
                    _ => Rc::new(indexmap::IndexMap::new()),
                };
                let values = Rc::make_mut(&mut object);
                values.reserve(fields.len());
                for (key, _) in fields {
                    values.entry(*key).or_insert(Value::Null);
                }
                for (key, field) in fields.iter().rev() {
                    *values.get_mut(key).unwrap() = build_value(field);
                }
                result = Value::Object(object);
            }
        }
        self.input = if let Some(left) = fallback {
            scalar::add_owned(&mut [left, result])?
        } else {
            result
        }
        .into();
        Ok(())
    }
}
