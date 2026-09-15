use act_sdk::prelude::*;

// `Source` is unused until Task 4 wires it into the `decode` tool.
#[allow(dead_code)]
mod source;
#[allow(unused_imports)]
use source::Source;

#[act_component]
mod component {
    use super::*;

    #[act_tool(description = "Say hello", read_only)]
    fn hello(
        /// Name to greet
        name: Option<String>,
    ) -> ActResult<String> {
        let who = name.unwrap_or_else(|| "world".to_string());
        Ok(format!("Hello, {who}!"))
    }
}
