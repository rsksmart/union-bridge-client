use bitcoin::Txid;
use common::msg_broker::bitvmx_types::ParticipantRole;
use common::types::{CommitteeId, StreamId};
use serde::{Deserialize, Serialize};

use crate::types::Utxo;
// TODO create types mod and move this and types.rs (renamed to rsk_events.rs) there

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ApplyToStream {
    pub stream_id: StreamId, // Matches StreamDenomination in the contract
    pub role: ParticipantRole,
    pub funding_utxo: Utxo,
    pub speed_up_utxo: Utxo,
    pub advance_funds: Utxo,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RejectPeginRequest {
    #[serde(with = "common::types::committee_id_decimal_string")]
    pub committee_id: CommitteeId,
    pub member_index: usize,
    #[serde(with = "common::types::txid_hex_string_optional_0x")]
    pub request_pegin_txid: Txid,
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn reject_pegin_request_serializes_large_committee_id_as_string() {
        let request = RejectPeginRequest {
            committee_id: CommitteeId::from(
                51_085_888_409_946_378_723_362_565_688_172_080_613_u128,
            ),
            member_index: 0,
            request_pegin_txid: "80abbaab55e259d922faad697287620a0069464acf50badd9214b6c123789d31"
                .parse()
                .expect("valid txid"),
        };

        let value = serde_json::to_value(request).expect("serialize request");

        assert_eq!(value["committee_id"], json!("51085888409946378723362565688172080613"));
    }

    #[test]
    fn reject_pegin_request_deserializes_committee_id_from_string_only() {
        let plain_txid = "80abbaab55e259d922faad697287620a0069464acf50badd9214b6c123789d31";
        let prefixed_txid = "0x80abbaab55e259d922faad697287620a0069464acf50badd9214b6c123789d31";
        let plain_value = json!({
            "committee_id": "51085888409946378723362565688172080613",
            "member_index": 1,
            "request_pegin_txid": plain_txid,
        });
        let prefixed_value = json!({
            "committee_id": "51085888409946378723362565688172080613",
            "member_index": 1,
            "request_pegin_txid": prefixed_txid,
        });

        let plain_request: RejectPeginRequest =
            serde_json::from_value(plain_value).expect("deserialize plain txid request");
        let prefixed_request: RejectPeginRequest =
            serde_json::from_value(prefixed_value).expect("deserialize prefixed txid request");

        assert_eq!(
            plain_request.committee_id,
            CommitteeId::from(51_085_888_409_946_378_723_362_565_688_172_080_613_u128)
        );
        assert_eq!(plain_request.request_pegin_txid, prefixed_request.request_pegin_txid);
    }

    #[test]
    fn reject_pegin_request_rejects_numeric_committee_id() {
        let txid = "80abbaab55e259d922faad697287620a0069464acf50badd9214b6c123789d31";
        let numeric_value = json!({
            "committee_id": 42,
            "member_index": 0,
            "request_pegin_txid": txid,
        });

        let err = serde_json::from_value::<RejectPeginRequest>(numeric_value)
            .expect_err("numeric committee_id should be rejected");

        assert!(err.to_string().contains("string"));
    }
}
