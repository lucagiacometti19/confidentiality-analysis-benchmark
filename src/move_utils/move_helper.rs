use super::move_parser::{MoveField, MovePrimitiveType, MoveStruct};

pub fn create_spec(move_struct: &MoveStruct, selected_field: &MoveField) -> Option<String> {
    let val = match selected_field._type.as_str() {
        "u8" | "u16" | "u32" | "u64" | "u128" | "u256" => MovePrimitiveType::Integer,
        "bool" => MovePrimitiveType::Bool,
        "address" => MovePrimitiveType::Address,
        _ => {
            error!(
                "Couldn't identify move struct field type:\n{:#?}",
                selected_field
            );
            return None;
        }
    };
    Some(format!(
        "    spec {} {{\n        invariant {} == {};\n    }}\n",
        move_struct.name,
        selected_field.name,
        val.default()
    ))
}
