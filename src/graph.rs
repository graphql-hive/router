pub struct Node {
    pub index: i16,
    pub type_name: String,
    pub graph_id: String,
    pub graph_name: String,
    pub is_leaf: bool,
}

pub struct Edge<T: Procedure> {
    pub head: Node,
    pub procedure: T,
    pub tail: Node,
}

pub trait Procedure {
    fn to_string(&self) -> String;
}

pub struct FieldMove {
    pub type_name: String,
    pub field_name: String,
    pub requires: Option<String>,
    pub provides: Option<String>,
    pub provided: bool,
}

impl Procedure for FieldMove {
    fn to_string(&self) -> String {
        let mut str = self.field_name.clone();

        // if let Some(requires) = self.requires {
        //     // str.insert_str(idx, string);
        //     str += &format!(" @require({})", requires);
        // }

        // if let Some(provides) = self.provides {
        //     str += &format!(" @provides({})", provides);
        // }

        // if self.provided {
        //     str += " @provided";
        // }

        return str;
    }
}

pub struct AbstractMove {
    pub key_fields: String,
}

impl Procedure for AbstractMove {
    fn to_string(&self) -> String {
        return format!("🔮 🔑 {}", self.key_fields);
    }
}

pub struct EntityMove {
    pub key_fields: String,
}

impl Procedure for EntityMove {
    fn to_string(&self) -> String {
        return format!("🔑 {}", self.key_fields);
    }
}
