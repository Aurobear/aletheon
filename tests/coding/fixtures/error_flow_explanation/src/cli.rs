use crate::domain::PortError;
pub fn exit_code(e:&PortError)->i32 { match e { PortError::Zero=>2, PortError::Invalid=>3 } }
pub fn render(e:&PortError)->&'static str { match e { PortError::Zero=>"port must be non-zero", PortError::Invalid=>"invalid port" } }
