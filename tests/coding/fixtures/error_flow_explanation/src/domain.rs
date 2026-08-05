#[derive(Debug)] pub enum PortError { Zero, Invalid }
pub fn parse_port(v:&str)->Result<u16,PortError>{ let p=v.parse().map_err(|_|PortError::Invalid)?; if p==0 {Err(PortError::Zero)} else {Ok(p)} }
