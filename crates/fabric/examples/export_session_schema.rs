fn main() {
    let schema = schemars::schema_for!(fabric::SessionProtocolV5);
    println!(
        "{}",
        serde_json::to_string_pretty(&schema).expect("serialize schema")
    );
}
