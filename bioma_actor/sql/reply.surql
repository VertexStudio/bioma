CREATE reply $reply_id CONTENT $reply;
RELATE $msg_id -> message_replies -> $reply_id;