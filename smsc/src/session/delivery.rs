use rusmpp::pdus::{DeliverSm, SubmitSm};
use rusmpp::tlvs::MessageDeliveryRequestTlvValue;
use rusmpp::types::{COctetString, EmptyOrFullCOctetString, OctetString};
use rusmpp::values::{EsmClass, MCDeliveryReceipt, MessageState, MessageType};

use crate::queue::QueueMessage;

pub(super) fn wants_delivery_receipt(submit: &SubmitSm) -> bool {
    !matches!(
        submit.registered_delivery.mc_delivery_receipt(),
        MCDeliveryReceipt::NoMcDeliveryReceiptRequested
    )
}

pub(super) fn build_deliver_sm(message: &QueueMessage) -> DeliverSm {
    let submit = &message.submit;

    DeliverSm::builder()
        .service_type(submit.service_type.clone())
        .source_addr_ton(submit.source_addr_ton)
        .source_addr_npi(submit.source_addr_npi)
        .source_addr(submit.source_addr.clone())
        .dest_addr_ton(submit.dest_addr_ton)
        .dest_addr_npi(submit.dest_addr_npi)
        .destination_addr(submit.destination_addr.clone())
        .esm_class(submit.esm_class)
        .protocol_id(submit.protocol_id)
        .priority_flag(submit.priority_flag)
        .schedule_delivery_time(submit.schedule_delivery_time.clone())
        .validity_period(submit.validity_period.clone())
        .registered_delivery(Default::default())
        .replace_if_present_flag(submit.replace_if_present_flag)
        .data_coding(submit.data_coding)
        .sm_default_msg_id(submit.sm_default_msg_id)
        .short_message(submit.short_message().clone())
        .build()
}

pub(super) fn build_delivery_receipt(
    submit: &SubmitSm,
    message_id: &COctetString<1, 65>,
) -> Result<DeliverSm, crate::session::SessionError> {
    let esm_class = EsmClass {
        message_type: MessageType::ShortMessageContainsMCDeliveryReceipt,
        ..EsmClass::default()
    };

    let receipt_text = format!("id:{} stat:DELIVRD", message_id.as_str());
    let short_message = OctetString::from_slice(receipt_text.as_bytes())
        .map_err(|_| crate::session::SessionError::ReceiptOverflow)?;

    Ok(DeliverSm::builder()
        .service_type(submit.service_type.clone())
        .source_addr_ton(submit.dest_addr_ton)
        .source_addr_npi(submit.dest_addr_npi)
        .source_addr(submit.destination_addr.clone())
        .dest_addr_ton(submit.source_addr_ton)
        .dest_addr_npi(submit.source_addr_npi)
        .destination_addr(submit.source_addr.clone())
        .esm_class(esm_class)
        .protocol_id(0)
        .priority_flag(submit.priority_flag)
        .schedule_delivery_time(EmptyOrFullCOctetString::empty())
        .validity_period(EmptyOrFullCOctetString::empty())
        .registered_delivery(Default::default())
        .replace_if_present_flag(submit.replace_if_present_flag)
        .data_coding(submit.data_coding)
        .sm_default_msg_id(0)
        .short_message(short_message)
        .push_tlv(MessageDeliveryRequestTlvValue::ReceiptedMessageId(
            message_id.clone(),
        ))
        .push_tlv(MessageDeliveryRequestTlvValue::MessageState(
            MessageState::Delivered,
        ))
        .build())
}
