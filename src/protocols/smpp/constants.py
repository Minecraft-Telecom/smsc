"""SMPP Protocol Constants and Enumerations."""
from enum import IntEnum


class CommandId(IntEnum):
    """SMPP Command IDs (PDU types)."""
    # Session Management
    GENERIC_NACK = 0x80000000
    BIND_RECEIVER = 0x00000001
    BIND_RECEIVER_RESP = 0x80000001
    BIND_TRANSMITTER = 0x00000002
    BIND_TRANSMITTER_RESP = 0x80000002
    BIND_TRANSCEIVER = 0x00000009
    BIND_TRANSCEIVER_RESP = 0x80000009
    UNBIND = 0x00000006
    UNBIND_RESP = 0x80000006
    OUTBIND = 0x0000000B
    
    # Message Submission
    SUBMIT_SM = 0x00000004
    SUBMIT_SM_RESP = 0x80000004
    SUBMIT_MULTI = 0x00000021
    SUBMIT_MULTI_RESP = 0x80000021
    
    # Message Delivery
    DELIVER_SM = 0x00000005
    DELIVER_SM_RESP = 0x80000005
    DATA_SM = 0x00000103
    DATA_SM_RESP = 0x80000103
    
    # Query/Cancel/Replace
    QUERY_SM = 0x00000003
    QUERY_SM_RESP = 0x80000003
    CANCEL_SM = 0x00000008
    CANCEL_SM_RESP = 0x80000008
    REPLACE_SM = 0x00000007
    REPLACE_SM_RESP = 0x80000007
    
    # Enquire Link (keepalive)
    ENQUIRE_LINK = 0x00000015
    ENQUIRE_LINK_RESP = 0x80000015
    
    # Alerts
    ALERT_NOTIFICATION = 0x00000102


class CommandStatus(IntEnum):
    """SMPP Command Status codes."""
    ESME_ROK = 0x00000000  # No Error
    ESME_RINVMSGLEN = 0x00000001  # Invalid Message Length
    ESME_RINVCMDLEN = 0x00000002  # Invalid Command Length
    ESME_RINVCMDID = 0x00000003  # Invalid Command ID
    ESME_RINVBNDSTS = 0x00000004  # Invalid Bind Status
    ESME_RALYBND = 0x00000005  # Already Bound
    ESME_RINVPRTFLG = 0x00000006  # Invalid Priority Flag
    ESME_RINVREGDLVFLG = 0x00000007  # Invalid Registered Delivery Flag
    ESME_RSYSERR = 0x00000008  # System Error
    ESME_RINVSRCADR = 0x0000000A  # Invalid Source Address
    ESME_RINVDSTADR = 0x0000000B  # Invalid Destination Address
    ESME_RINVMSGID = 0x0000000C  # Invalid Message ID
    ESME_RBINDFAIL = 0x0000000D  # Bind Failed
    ESME_RINVPASWD = 0x0000000E  # Invalid Password
    ESME_RINVSYSID = 0x0000000F  # Invalid System ID
    ESME_RCANCELFAIL = 0x00000011  # Cancel SM Failed
    ESME_RREPLACEFAIL = 0x00000013  # Replace SM Failed
    ESME_RMSGQFUL = 0x00000014  # Message Queue Full
    ESME_RINVSERTYP = 0x00000015  # Invalid Service Type
    ESME_RINVNUMDESTS = 0x00000033  # Invalid Number of Destinations
    ESME_RINVDLNAME = 0x00000034  # Invalid Distribution List Name
    ESME_RINVDESTFLAG = 0x00000040  # Invalid Destination Flag
    ESME_RINVSUBREP = 0x00000042  # Invalid Submit With Replace
    ESME_RINVESMCLASS = 0x00000043  # Invalid ESM Class
    ESME_RCNTSUBDL = 0x00000044  # Cannot Submit to Distribution List
    ESME_RSUBMITFAIL = 0x00000045  # Submit SM Failed
    ESME_RINVSRCTON = 0x00000048  # Invalid Source TON
    ESME_RINVSRCNPI = 0x00000049  # Invalid Source NPI
    ESME_RINVDSTTON = 0x00000050  # Invalid Destination TON
    ESME_RINVDSTNPI = 0x00000051  # Invalid Destination NPI
    ESME_RINVSYSTYP = 0x00000053  # Invalid System Type
    ESME_RINVREPFLAG = 0x00000054  # Invalid Replace If Present Flag
    ESME_RINVNUMMSGS = 0x00000055  # Invalid Number of Messages
    ESME_RTHROTTLED = 0x00000058  # Throttling Error
    ESME_RINVSCHED = 0x00000061  # Invalid Scheduled Delivery Time
    ESME_RINVEXPIRY = 0x00000062  # Invalid Validity Period
    ESME_RINVDFTMSGID = 0x00000063  # Predefined Message Invalid
    ESME_RX_T_APPN = 0x00000064  # ESME Receiver Temporary App Error
    ESME_RX_P_APPN = 0x00000065  # ESME Receiver Permanent App Error
    ESME_RX_R_APPN = 0x00000066  # ESME Receiver Reject Message Error
    ESME_RQUERYFAIL = 0x00000067  # Query SM Failed
    ESME_RINVTLVSTREAM = 0x000000C0  # TLV Stream Error
    ESME_RTLVNOTALLWD = 0x000000C1  # TLV Not Allowed
    ESME_RINVTLVLEN = 0x000000C2  # Invalid TLV Length
    ESME_RMISSINGTLV = 0x000000C3  # Expected TLV Missing
    ESME_RINVTLVVAL = 0x000000C4  # Invalid TLV Value
    ESME_RDELIVERYFAILURE = 0x000000FE  # Delivery Failure
    ESME_RUNKNOWNERR = 0x000000FF  # Unknown Error


class TON(IntEnum):
    """Type of Number."""
    UNKNOWN = 0x00
    INTERNATIONAL = 0x01
    NATIONAL = 0x02
    NETWORK_SPECIFIC = 0x03
    SUBSCRIBER_NUMBER = 0x04
    ALPHANUMERIC = 0x05
    ABBREVIATED = 0x06


class NPI(IntEnum):
    """Numbering Plan Indicator."""
    UNKNOWN = 0x00
    ISDN = 0x01  # E.163/E.164
    DATA = 0x03  # X.121
    TELEX = 0x04  # F.69
    LAND_MOBILE = 0x06  # E.212
    NATIONAL = 0x08
    PRIVATE = 0x09
    ERMES = 0x0A
    INTERNET = 0x0E  # IP
    WAP_CLIENT_ID = 0x12


class DataCoding(IntEnum):
    """Data Coding Scheme."""
    DEFAULT = 0x00  # SMSC Default Alphabet (GSM 7-bit)
    IA5 = 0x01  # IA5 (CCITT T.50)/ASCII
    BINARY_8BIT = 0x02  # Octet unspecified (8-bit binary)
    LATIN1 = 0x03  # Latin 1 (ISO-8859-1)
    BINARY = 0x04  # Octet unspecified (8-bit binary)
    JIS = 0x05  # JIS (X 0208-1990)
    CYRILLIC = 0x06  # Cyrillic (ISO-8859-5)
    LATIN_HEBREW = 0x07  # Latin/Hebrew (ISO-8859-8)
    UCS2 = 0x08  # UCS2 (ISO/IEC-10646)
    PICTOGRAM = 0x09  # Pictogram Encoding
    ISO_2022_JP = 0x0A  # ISO-2022-JP (Music Codes)
    KANJI = 0x0D  # Extended Kanji JIS
    KS_C_5601 = 0x0E  # KS C 5601


class ESMClass(IntEnum):
    """ESM Class messaging mode."""
    DEFAULT = 0x00
    DATAGRAM = 0x01
    TRANSACTION = 0x02
    STORE_AND_FORWARD = 0x03


# SMPP Header size in bytes
SMPP_HEADER_SIZE = 16
SMPP_MIN_PDU_SIZE = SMPP_HEADER_SIZE
SMPP_MAX_PDU_SIZE = 65535

# Default SMPP version
SMPP_VERSION = 0x34  # SMPP v3.4
