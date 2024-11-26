LET $reply_id = CREATE ONLY reply CONTENT $reply;
RELATE $msg_id -> message_replies -> $reply_id;