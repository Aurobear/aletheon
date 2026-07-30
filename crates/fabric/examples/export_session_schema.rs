fn main() {
    let schema = schemars::schema_for!(fabric::SessionProtocolV4);
    println!(
        "{}",
        serde_json::to_string_pretty(&schema).expect("serialize schema")
    );
}
