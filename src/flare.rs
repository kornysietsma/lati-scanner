#![warn(clippy::all)]

use serde::ser::SerializeStruct;
use serde::{Serialize, Serializer};
use std::collections::HashMap;
use std::ffi::OsString;

#[derive(Debug, PartialEq)]
pub struct FlareTreeNode {
    name: OsString,
    is_file: bool,
    children: Vec<FlareTreeNode>,
    data: HashMap<String, serde_json::Value>,
}

impl FlareTreeNode {
    #[allow(dead_code)]
    pub fn name(&self) -> &OsString {
        &self.name
    }

    pub fn new<S: Into<OsString>>(name: S, is_file: bool) -> FlareTreeNode {
        FlareTreeNode {
            name: name.into(),
            is_file,
            children: Vec::new(),
            data: HashMap::new(),
        }
    }

    pub fn add_data<S: Into<String>>(&mut self, key: S, value: serde_json::Value) {
        self.data.insert(key.into(), value); // TODO: should we return what insert returns? Or self?
    }

    pub fn append_child(&mut self, child: FlareTreeNode) {
        if self.is_file {
            panic!("appending child to a directory: {:?}", self)
        }
        self.children.push(child); // TODO - return self?
    }

    /// gets a tree entry by path, or None if something along the path doesn't exist
    #[allow(dead_code)]
    pub fn get_in(&self, path: &mut std::path::Components) -> Option<&FlareTreeNode> {
        match path.next() {
            Some(first_name) => {
                let dir_name = first_name.as_os_str();
                if !self.is_file {
                    let first_match = self.children.iter().find(|c| dir_name == c.name)?;
                    return first_match.get_in(path);
                }
                None
            }
            None => Some(self),
        }
    }

    /// gets a mutable tree entry by path, or None if something along the path doesn't exist
    pub fn get_in_mut(&mut self, path: &mut std::path::Components) -> Option<&mut FlareTreeNode> {
        match path.next() {
            Some(first_name) => {
                let dir_name = first_name.as_os_str();
                if !self.is_file {
                    let first_match = self.children.iter_mut().find(|c| dir_name == c.name)?;
                    return first_match.get_in_mut(path);
                }
                None
            }
            None => Some(self),
        }
    }
}

impl Serialize for FlareTreeNode {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut state = serializer.serialize_struct("FlareTreeNode", 2)?;
        let name_str = self.name.to_str().expect("Can't serialize!"); // TODO: how to convert to error result?
        state.serialize_field("name", &name_str)?;
        if !self.data.is_empty() {
            state.serialize_field("data", &self.data)?
        }
        if !self.is_file {
            state.serialize_field("children", &self.children)?;
        }

        state.end()
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use regex::Regex;
    use serde_json::json;
    use serde_json::Value;
    use std::path::Path;

    #[test]
    fn can_build_tree() {
        let mut root = FlareTreeNode::new("root", false);
        root.append_child(FlareTreeNode::new("child", true));

        assert_eq!(
            root,
            FlareTreeNode {
                name: OsString::from("root"),
                is_file: false,
                children: vec![FlareTreeNode {
                    name: OsString::from("child"),
                    is_file: true,
                    data: HashMap::new(),
                    children: Vec::new()
                }],
                data: HashMap::new()
            }
        )
    }

    fn build_test_tree() -> FlareTreeNode {
        let mut root = FlareTreeNode::new("root", false);
        root.append_child(FlareTreeNode::new("root_file_1.txt", true));
        root.append_child(FlareTreeNode::new("root_file_2.txt", true));
        let mut child1 = FlareTreeNode::new("child1", false);
        child1.append_child(FlareTreeNode::new("child1_file_1.txt", true));
        let mut grand_child = FlareTreeNode::new("grandchild", false);
        grand_child.append_child(FlareTreeNode::new("grandchild_file.txt", true));
        child1.append_child(grand_child);
        child1.append_child(FlareTreeNode::new("child1_file_2.txt", true));
        let mut child2 = FlareTreeNode::new("child2", false);
        child2.add_data("meta", json!("wibble"));
        let mut child2_file = FlareTreeNode::new("child2_file.txt", true);
        let widget_data = json!({
            "sprockets": 7,
            "flanges": ["Nigel, Sarah"]
        });
        child2_file.add_data("widgets", widget_data);
        child2.append_child(child2_file);
        root.append_child(child1);
        root.append_child(child2);
        root
    }

    #[test]
    fn can_get_elements_from_tree() {
        let tree = build_test_tree();

        let mut path = std::path::Path::new("child1/grandchild/grandchild_file.txt").components();
        let grandchild = tree.get_in(&mut path);
        assert_eq!(
            grandchild.expect("Grandchild not found!").name(),
            "grandchild_file.txt"
        );
    }

    #[test]
    fn can_get_top_level_element_from_tree() {
        let tree = build_test_tree();

        let mut path = std::path::Path::new("child1").components();
        let child1 = tree.get_in(&mut path);
        assert_eq!(child1.expect("child1 not found!").name(), "child1");

        let mut path2 = std::path::Path::new("root_file_1.txt").components();
        let child2 = tree.get_in(&mut path2);
        assert_eq!(
            child2.expect("root_file_1 not found!").name(),
            "root_file_1.txt"
        );
    }

    #[test]
    fn getting_missing_elements_returns_none() {
        let tree = build_test_tree();
        let mut path = std::path::Path::new("child1/grandchild/nonesuch").components();
        let missing = tree.get_in(&mut path);
        assert_eq!(missing.is_none(), true);

        let mut path2 =
            Path::new("child1/grandchild/grandchild_file.txt/files_have_no_kids").components();
        let missing2 = tree.get_in(&mut path2);
        assert_eq!(missing2.is_none(), true);

        let mut path3 = Path::new("no_file_at_root").components();
        let missing3 = tree.get_in(&mut path3);
        assert_eq!(missing3.is_none(), true);
    }

    #[test]
    fn can_get_mut_elements_from_tree() {
        let mut tree = build_test_tree();
        let grandchild = tree
            .get_in_mut(&mut Path::new("child1/grandchild/grandchild_file.txt").components())
            .expect("Grandchild not found!");
        assert_eq!(grandchild.name(), "grandchild_file.txt");
        grandchild.name = OsString::from("fish");
        let grandchild2 = tree.get_in_mut(&mut Path::new("child1/grandchild/fish").components());
        assert_eq!(grandchild2.expect("fish not found!").name(), "fish");

        let grandchild_dir = tree
            .get_in_mut(&mut Path::new("child1/grandchild").components())
            .expect("Grandchild dir not found!");
        assert_eq!(grandchild_dir.name(), "grandchild");
        grandchild_dir.append_child(FlareTreeNode::new("new_kid_on_the_block.txt", true));
        let new_kid = tree
            .get_in_mut(&mut Path::new("child1/grandchild/new_kid_on_the_block.txt").components())
            .expect("New kid not found!");
        assert_eq!(new_kid.name(), "new_kid_on_the_block.txt");
    }

    #[test]
    fn can_get_json_payloads_from_tree() {
        let tree = build_test_tree();
        let file = tree
            .get_in(&mut Path::new("child2/child2_file.txt").components())
            .unwrap();

        assert_eq!(file.name(), "child2_file.txt");

        let expected = json!({
            "sprockets": 7,
            "flanges": ["Nigel, Sarah"]
        });

        assert_eq!(&file.data["widgets"], &expected);
    }

    fn strip(string: &str) -> String {
        let re = Regex::new(r"\s+").unwrap();
        re.replace_all(string, "").to_string()
    }

    #[test]
    fn can_serialize_directory_to_json() {
        let root = FlareTreeNode::new("root", false);

        let serialized = serde_json::to_string(&root).unwrap();

        assert_eq!(
            serialized,
            strip(
                r#"{
                    "name":"root",
                    "children": []
                }"#
            )
        )
    }

    #[test]
    fn can_serialize_dir_with_data_to_json() {
        let mut dir = FlareTreeNode::new("foo", false);
        dir.add_data("wibble", json!("fnord"));

        let serialized = serde_json::to_string(&dir).unwrap();

        assert_eq!(
            serialized,
            strip(
                r#"{
                    "name":"foo",
                    "data": {"wibble":"fnord"},
                    "children": []
                }"#
            )
        )
    }

    #[test]
    fn can_serialize_file_to_json() {
        let file = FlareTreeNode::new("foo.txt", true);

        let serialized = serde_json::to_string(&file).unwrap();

        assert_eq!(
            serialized,
            strip(
                r#"{
                    "name":"foo.txt"
                }"#
            )
        )
    }

    #[test]
    fn can_serialize_file_with_data_to_json() {
        let mut file = FlareTreeNode::new("foo.txt", true);
        file.add_data("wibble", json!("fnord"));

        let serialized = serde_json::to_string(&file).unwrap();

        assert_eq!(
            serialized,
            strip(
                r#"{
                    "name":"foo.txt",
                    "data": {"wibble":"fnord"}
                }"#
            )
        )
    }

    #[test]
    fn can_serialize_file_with_data_value_to_json() {
        let mut file = FlareTreeNode::new("foo.txt", true);
        let value = json!({"foo": ["bar", "baz", 123]});
        file.add_data("bat", value);

        let serialized = serde_json::to_string(&file).unwrap();

        assert_eq!(
            serialized,
            strip(
                r#"{
                    "name":"foo.txt",
                    "data": {"bat": {"foo": ["bar", "baz", 123]}}
                }"#
            )
        )
    }

    #[test]
    fn can_serialize_simple_tree_to_json() {
        let mut root = FlareTreeNode::new("root", false);
        root.append_child(FlareTreeNode::new("child.txt", true));
        root.append_child(FlareTreeNode::new("child2", false));

        let serialized = serde_json::to_string(&root).unwrap();
        let reparsed: Value = serde_json::from_str(&serialized).unwrap();

        assert_eq!(
            reparsed,
            json!({
                "name":"root",
                "children":[
                    {
                        "name": "child.txt"
                    },
                    {
                        "name":"child2",
                        "children":[]
                    }
                ]
            })
        )
    }
}
