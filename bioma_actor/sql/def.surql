-- ------------------------------
-- TABLE: actor
-- ------------------------------

DEFINE TABLE actor TYPE ANY SCHEMALESS PERMISSIONS NONE;

-- ------------------------------
-- TABLE: message
-- ------------------------------

DEFINE TABLE message TYPE NORMAL SCHEMALESS PERMISSIONS NONE;

DEFINE FIELD name ON message TYPE string PERMISSIONS FULL;
DEFINE FIELD rx ON message TYPE record<actor> PERMISSIONS FULL;
DEFINE FIELD tx ON message TYPE record<actor> PERMISSIONS FULL;

-- ------------------------------
-- TABLE: reply
-- ------------------------------

DEFINE TABLE reply TYPE NORMAL SCHEMALESS PERMISSIONS NONE;

DEFINE FIELD name ON reply TYPE string PERMISSIONS FULL;
DEFINE FIELD rx ON reply TYPE record<actor> PERMISSIONS FULL;
DEFINE FIELD tx ON reply TYPE record<actor> PERMISSIONS FULL;

-- ------------------------------
-- TABLE: message_replies
-- ------------------------------

DEFINE TABLE message_replies TYPE RELATION IN message OUT reply SCHEMALESS PERMISSIONS NONE;

DEFINE FIELD in ON message_replies TYPE record<message> PERMISSIONS FULL;
DEFINE FIELD out ON message_replies TYPE record<reply> PERMISSIONS FULL;

-- ------------------------------
-- TABLE: health
-- ------------------------------

DEFINE TABLE health TYPE NORMAL SCHEMALESS PERMISSIONS NONE;

DEFINE FIELD last_seen ON health TYPE datetime PERMISSIONS FULL;
DEFINE FIELD enabled ON health TYPE bool PERMISSIONS FULL;
DEFINE FIELD update_interval ON health TYPE duration PERMISSIONS FULL;

-- ------------------------------
-- TABLE: actor_health
-- ------------------------------

DEFINE TABLE actor_health TYPE RELATION IN actor OUT health SCHEMALESS PERMISSIONS NONE;

DEFINE FIELD in ON actor_health TYPE record<actor> PERMISSIONS FULL;
DEFINE FIELD out ON actor_health TYPE record<health> PERMISSIONS FULL;
