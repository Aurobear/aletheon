use crate::domain::Priority; pub fn parse(v:&str)->Result<Priority,String>{Ok(Priority(v.into()))}
