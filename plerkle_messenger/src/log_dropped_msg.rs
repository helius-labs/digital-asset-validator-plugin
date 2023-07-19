use crate::{ACCOUNT_STREAM, TRANSACTION_STREAM};
use plerkle_serialization::{
    error::PlerkleSerializationError, root_as_account_info, root_as_transaction_info,
};

pub fn extract_id(bytes: &Vec<u8>, stream_key: &str) -> Result<String, PlerkleSerializationError> {
    match stream_key {
        TRANSACTION_STREAM => match root_as_transaction_info(bytes) {
            Ok(tx) => {
                let signature = tx.signature().unwrap_or("NO SIG");
                Ok(signature.to_string())
            }
            Err(_) => Err(PlerkleSerializationError::SerializationError(
                "Could not serialize txn".to_string(),
            )),
        },
        ACCOUNT_STREAM => match root_as_account_info(bytes) {
            Ok(acc) => {
                if let Some(pubkey) = acc.pubkey() {
                    let pubkey_string = bs58::encode(pubkey.0.as_slice()).into_string();
                    Ok(pubkey_string)
                } else {
                    Ok("NO PUBKEY".to_string())
                }
            }
            Err(_) => Err(PlerkleSerializationError::SerializationError(
                "Could not serialize acc".to_string(),
            )),
        },
        _ => Ok("Neither TXN nor ACC stream".to_string()),
    }
}
