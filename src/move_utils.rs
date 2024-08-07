pub mod move_parser {
    use std::collections::HashSet;

    pub enum MovePrimitiveType {
        Integer,
        Bool,
        Address,
    }

    impl MovePrimitiveType {
        pub fn default(&self) -> &str {
            match *self {
                MovePrimitiveType::Integer => "0",
                MovePrimitiveType::Bool => "true",
                MovePrimitiveType::Address => "@0xCAFE",
            }
        }
    }

    #[derive(Debug, PartialEq, Eq, Hash)]
    pub struct MoveStruct {
        pub name: String,
        // (start, finish)
        pub declaration_span: (usize, usize),
        pub fields: Vec<MoveField>,
    }

    #[derive(Debug, PartialEq, Eq, Hash)]
    pub struct MoveField {
        pub name: String,
        pub _type: String,
    }

    #[derive(Debug, PartialEq, Eq, Hash)]
    pub struct MoveModule {
        pub address: String,
        pub name: String,
        pub declaration_line: usize,
    }

    #[derive(Debug, PartialEq, Eq, Default)]
    pub struct MoveSourceMetadata {
        pub modules: HashSet<MoveModule>,
        pub structs: HashSet<MoveStruct>,
    }

    impl MoveSourceMetadata {
        pub fn parse_move_source_metadata(
            code: std::borrow::Cow<str>,
        ) -> Result<MoveSourceMetadata, String> {
            let mut move_source_metadata = MoveSourceMetadata::default();
            let mut current_struct: Option<MoveStruct> = None;
            let mut current_fields: Vec<MoveField> = Vec::new();

            for (idx, line) in code.lines().enumerate() {
                let trimmed_line = line.trim();
                // Check for module definition
                if trimmed_line.starts_with("module") {
                    let module_path = trimmed_line
                        .split_whitespace()
                        .nth(1)
                        .unwrap()
                        .split_once("::")
                        .unwrap();
                    move_source_metadata.modules.insert(MoveModule {
                        address: module_path.0.to_owned(),
                        name: module_path.1.to_owned(),
                        declaration_line: idx,
                    });
                }
                // Check for struct definition start
                else if trimmed_line.starts_with("struct") || trimmed_line.ends_with("}") {
                    if trimmed_line.starts_with("struct") {
                        let struct_name = trimmed_line
                            .split_whitespace()
                            .nth(1)
                            .unwrap()
                            .split("<")
                            .next()
                            .unwrap();
                        current_struct = Some(MoveStruct {
                            name: struct_name.to_string(),
                            declaration_span: (idx, 0),
                            fields: vec![],
                        });
                        current_fields = Vec::new();
                    }
                    if trimmed_line.ends_with("}") {
                        if let Some(mut move_struct) = current_struct {
                            move_struct.declaration_span.1 = idx;
                            move_struct.fields = current_fields;
                            move_source_metadata.structs.insert(move_struct);
                        }
                        current_struct = None;
                        current_fields = Vec::new();
                    }
                } else if !trimmed_line.is_empty()
                    && !trimmed_line.starts_with("//")
                    && current_struct.is_some()
                {
                    // Extract field name and type (heuristic approach)
                    let parts = trimmed_line.split_once(":").unwrap();
                    let field_name = parts.0.trim().to_string();
                    let field_type = if parts.1.trim().ends_with(',') {
                        let mut tmp_chars = parts.1.trim().chars();
                        tmp_chars.next_back();
                        tmp_chars.as_str()
                    } else {
                        parts.1.trim()
                    };
                    current_fields.push(MoveField {
                        name: field_name,
                        _type: field_type.to_owned(),
                    });
                }
            }

            if current_struct.is_some() {
                return Err(String::from("Unterminated struct definition"));
            }

            Ok(move_source_metadata)
        }
    }
}

pub mod move_helper;
