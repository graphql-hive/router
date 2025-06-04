/// [[String!]]! -> String
pub fn strip_modifiers_from_type_string(type_ref: &String) -> String {
    type_ref
        .trim_start_matches('[')
        .trim_end_matches([']', '!'])
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_modifiers() {
        assert_eq!(
            strip_modifiers_from_type_string(&String::from("String")),
            "String"
        );
        assert_eq!(
            strip_modifiers_from_type_string(&String::from("String")),
            "String"
        );
        assert_eq!(
            strip_modifiers_from_type_string(&String::from("[String]")),
            "String"
        );
        assert_eq!(
            strip_modifiers_from_type_string(&String::from("[[String]]")),
            "String"
        );
        assert_eq!(
            strip_modifiers_from_type_string(&String::from("[[String!]]!")),
            "String"
        );
    }
}
