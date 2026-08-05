use crate::{application::status,domain::DomainError}; pub fn response(e:DomainError)->(u16,&'static str){(status(e),"internal")}
