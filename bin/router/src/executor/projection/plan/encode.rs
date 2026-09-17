use super::ir::{Field, Value};
use super::tables::{id, TablesBuilder};
use super::*;

impl TypeGuardId {
    pub(super) fn from_index(index: usize) -> Self {
        Self(NonZeroU32::new(id(index) + 1).expect("index + 1 is never zero"))
    }
}

impl ConditionId {
    pub(super) fn from_index(index: usize) -> Self {
        Self(NonZeroU32::new(id(index) + 1).expect("index + 1 is never zero"))
    }
}

#[derive(Default)]
pub(super) struct Encoder {
    tables: TablesBuilder,
    fields: Vec<FieldRecord>,
}

impl Encoder {
    pub(super) fn encode(fields: &[Field<'_>]) -> ProjectionPlan {
        let mut encoder = Self::default();
        let roots = encoder.encode_range(fields);
        ProjectionPlan {
            roots,
            fields: encoder.fields.into_boxed_slice(),
            tables: Arc::new(ProjectionTables {
                text: encoder.tables.text.into_boxed_str(),
                symbols: encoder.tables.symbols.into_boxed_slice(),
                conditions: encoder.tables.conditions.into_boxed_slice(),
                guards: encoder.tables.guards.into_boxed_slice(),
                sets: encoder.tables.sets.into_boxed_slice(),
                set_members: encoder.tables.set_members.into_boxed_slice(),
                shape_ranges: encoder.tables.shape_ranges.into_boxed_slice(),
                shape_flags: encoder.tables.shape_flags.into_boxed_slice(),
            }),
        }
    }

    /// Encodes siblings first, then their children, so each sibling range stays
    /// contiguous in the flat field array and child ranges can safely reference it.
    fn encode_range(&mut self, fields: &[Field<'_>]) -> Range {
        let start = id(self.fields.len());
        for field in fields {
            let record = self.encode_field(field);
            self.fields.push(record);
        }
        for (offset, field) in fields.iter().enumerate() {
            if let Value::Children(sub_fields) = &field.value {
                self.fields[start as usize + offset].children = self.encode_range(sub_fields);
            }
        }
        Range {
            start,
            len: id(fields.len()),
        }
    }

    fn encode_field(&mut self, field: &Field<'_>) -> FieldRecord {
        let field_name = self.tables.intern_symbol(field.field_name);
        let response_key = if field.field_name == field.response_key {
            field_name
        } else {
            self.tables.intern_symbol(field.response_key)
        };

        let mut flags = 0;
        if matches!(field.value, Value::Children(_)) {
            flags |= FieldMeta::CHILDREN;
        }
        if field.is_typename {
            flags |= FieldMeta::TYPENAME;
        }
        if field.nullability.is_non_null() {
            flags |= FieldMeta::NON_NULL;
        }

        FieldRecord {
            field_name,
            response_key: self.tables.symbol_range(response_key),
            meta: FieldMeta::new(self.tables.intern_shape(field.nullability), flags),
            parent_guard: field
                .parent_scope
                .map(|type_set| self.tables.intern_guard(type_set)),
            condition: match field.condition {
                ir::Condition::Always => None,
                condition => Some(self.encode_condition(&condition)),
            },
            children: Range::default(),
        }
    }

    fn encode_condition(&mut self, condition: &ir::Condition) -> ConditionId {
        use ir::Condition::*;
        let record = match condition {
            Always => unreachable!("unconditional selection conditions are not encoded"),
            IncludeIf(var_name) => Condition::Include(self.tables.intern_symbol(var_name)),
            SkipIf(var_name) => Condition::Skip(self.tables.intern_symbol(var_name)),
            ParentType(types) => Condition::ParentType(self.tables.intern_guard(*types)),
            FieldType(types) => Condition::FieldType(self.tables.intern_guard(*types)),
            EnumValues(allowed) => Condition::EnumValues(self.tables.intern_members(*allowed)),
            And(left, right) => {
                Condition::And(self.encode_condition(left), self.encode_condition(right))
            }
            Or(left, right) => {
                Condition::Or(self.encode_condition(left), self.encode_condition(right))
            }
        };
        self.tables.intern_condition(record)
    }
}
