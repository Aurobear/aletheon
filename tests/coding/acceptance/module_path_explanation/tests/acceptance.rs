#[test]
fn report_identifies_entry_implementation_and_test() {
 let report=include_str!("../REPORT.md");
 for required in ["src/lib.rs", "src/normalize.rs", "tests/public.rs", "replace"] { assert!(report.contains(required), "missing {required}"); }
}
