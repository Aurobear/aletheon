use crate::store::ReceiptStore; pub fn resume(store:&ReceiptStore,id:&str,mut execute:impl FnMut()){ let _=store; let _=id; execute(); }
