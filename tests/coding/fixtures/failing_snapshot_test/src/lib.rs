pub fn render(id:u64,name:&str,active:bool)->String { format!("name={name};id={id};active={active}") }
#[cfg(test)] mod tests {use super::*; #[test] fn snapshot(){assert_eq!(render(7,"worker",true),"id=7;name=worker;active=true");}}
