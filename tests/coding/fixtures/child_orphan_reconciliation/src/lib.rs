#[derive(Clone,Copy,Debug,PartialEq,Eq)] pub enum State{Running,Succeeded,Failed,Orphaned}
#[derive(Clone,Copy)] pub struct Child{pub generation:u64,pub state:State}
pub fn reconcile(current:u64,child:Child,receipt:Option<State>)->State { let _=(current,receipt); child.state }
