//! Native function ID allocation.

// ---- Globals (§24.2) ----
pub const PRINT: usize = 1;
pub const WRITE: usize = 2;
pub const READ_LINE: usize = 3;
pub const READ_KEY: usize = 4;
pub const SLEEP: usize = 5;
pub const PARSE_INT: usize = 6;
pub const PARSE_FLOAT: usize = 7;
pub const TO_STRING: usize = 8;
pub const IS_NULL: usize = 9;
pub const IS_NOT_NULL: usize = 10;
pub const IS_NUMERIC: usize = 11;
pub const IS_ALPHA: usize = 12;
pub const IS_EMAIL: usize = 13;
pub const RANDOM_INT: usize = 14;
pub const GENERATE_GUID: usize = 15;
pub const GET_TIME: usize = 16;
pub const ASSERT: usize = 17;
pub const ASSERT_EQ: usize = 18;
pub const MALLOC: usize = 19;
pub const FREE: usize = 20;
pub const SIZEOF: usize = 21;
pub const REGEX_NEW: usize = 22;
pub const OK: usize = 23;
pub const ERR: usize = 24;

// ---- List / Array methods (receiver first) ----
pub const LIST_ADD: usize = 100;
pub const LIST_GET: usize = 101;
pub const LIST_REMOVE_AT: usize = 102;
pub const LIST_CONTAINS: usize = 103;
pub const LIST_CLEAR: usize = 104;
pub const LIST_SUM: usize = 105;
pub const LIST_AVERAGE: usize = 106;
pub const LIST_MAX: usize = 107;
pub const LIST_MIN: usize = 108;
pub const LIST_FIRST: usize = 109;
pub const LIST_LAST: usize = 110;
pub const LIST_ANY: usize = 111; // stub without callback
pub const LIST_ALL: usize = 112;
pub const LIST_WHERE: usize = 113; // needs function arg
pub const LIST_SELECT: usize = 114;
pub const LIST_PARALLEL_MAP: usize = 115;

// ---- Dict methods ----
pub const DICT_CONTAINS_KEY: usize = 130;
pub const DICT_REMOVE: usize = 131;
pub const DICT_CLEAR: usize = 132;
pub const DICT_KEYS: usize = 133;
pub const DICT_VALUES: usize = 134;

// ---- Set / Queue / Stack ----
pub const SET_ADD: usize = 140;
pub const SET_CONTAINS: usize = 141;
pub const SET_REMOVE: usize = 142;
pub const QUEUE_ENQUEUE: usize = 150;
pub const QUEUE_DEQUEUE: usize = 151;
pub const QUEUE_PEEK: usize = 152;
pub const STACK_PUSH: usize = 160;
pub const STACK_POP: usize = 161;
pub const STACK_PEEK: usize = 162;

// ---- String methods ----
pub const STR_TO_UPPER: usize = 200;
pub const STR_TO_LOWER: usize = 201;
pub const STR_TRIM: usize = 202;
pub const STR_TRIM_START: usize = 203;
pub const STR_TRIM_END: usize = 204;
pub const STR_CONTAINS: usize = 205;
pub const STR_STARTS_WITH: usize = 206;
pub const STR_ENDS_WITH: usize = 207;
pub const STR_INDEX_OF: usize = 208;
pub const STR_REPLACE: usize = 209;
pub const STR_SUBSTRING: usize = 210;
pub const STR_SPLIT: usize = 211;
pub const STR_JOIN: usize = 212; // static string.Join

// ---- StringBuilder ----
pub const SB_APPEND: usize = 220;
pub const SB_TO_STRING: usize = 221;
pub const SB_CLEAR: usize = 222;

// ---- Regex object methods ----
pub const RE_FIND_ALL: usize = 230;
pub const RE_IS_MATCH: usize = 231;
pub const RE_REPLACE: usize = 232;

// ---- Result methods / props via methods ----
pub const RESULT_IS_OK: usize = 240;
pub const RESULT_VALUE: usize = 241;
pub const RESULT_ERROR: usize = 242;

// ---- File static ----
pub const FILE_READ_TEXT: usize = 300;
pub const FILE_WRITE_TEXT: usize = 301;
pub const FILE_APPEND_TEXT: usize = 302;
pub const FILE_READ_BYTES: usize = 303;
pub const FILE_WRITE_BYTES: usize = 304;
pub const FILE_EXISTS: usize = 305;
pub const FILE_DELETE: usize = 306;
pub const FILE_GET_INFO: usize = 307;

// ---- Directory static ----
pub const DIR_GET_FILES: usize = 320;
pub const DIR_GET_DIRS: usize = 321;
pub const DIR_CREATE: usize = 322;
pub const DIR_DELETE: usize = 323;
pub const DIR_EXISTS: usize = 324;

// ---- Math static ----
pub const MATH_ABS: usize = 400;
pub const MATH_SQRT: usize = 401;
pub const MATH_POW: usize = 402;
pub const MATH_FLOOR: usize = 403;
pub const MATH_CEIL: usize = 404;
pub const MATH_ROUND: usize = 405;
pub const MATH_MIN: usize = 406;
pub const MATH_MAX: usize = 407;
pub const MATH_SIN: usize = 408;
pub const MATH_COS: usize = 409;
pub const MATH_TAN: usize = 410;
pub const MATH_LOG: usize = 411;
pub const MATH_EXP: usize = 412;
pub const MATH_PI: usize = 413;
pub const MATH_E: usize = 414;

// ---- Random ----
pub const RANDOM_NEXT: usize = 420;
pub const RANDOM_NEXT_DOUBLE: usize = 421;

// ---- DateTime ----
pub const DT_NOW: usize = 440;
pub const DT_UTC_NOW: usize = 441;
pub const DT_TO_STRING: usize = 442;

// ---- Json ----
pub const JSON_PARSE: usize = 500;
pub const JSON_STRINGIFY: usize = 501;
pub const JSON_SERIALIZE: usize = 502;
pub const JSON_DESERIALIZE: usize = 503;

// ---- Yaml ----
pub const YAML_PARSE: usize = 510;
pub const YAML_SERIALIZE: usize = 511;
pub const YAML_DESERIALIZE: usize = 512;

// ---- Hash ----
pub const HASH_SHA256: usize = 600;
pub const HASH_SHA1: usize = 601;
pub const HASH_MD5: usize = 602;

// ---- Http ----
pub const HTTP_GET: usize = 700;
pub const HTTP_POST: usize = 701;
pub const HTTP_GET_ASYNC: usize = 702; // sync under the hood

// ---- Tcp / Udp ----
pub const TCP_CONNECT: usize = 720;
pub const TCP_SEND: usize = 721;
pub const TCP_RECEIVE: usize = 722;
pub const TCP_CLOSE: usize = 723;
pub const UDP_SEND: usize = 730;
pub const UDP_RECEIVE: usize = 731;

// ---- Task ----
pub const TASK_DELAY: usize = 800;
pub const TASK_RUN: usize = 801;
pub const TASK_WHEN_ALL: usize = 802;

// ---- GC ----
pub const GC_COLLECT: usize = 810;
pub const GC_STATS: usize = 811;

// ---- Logger ----
pub const LOG_INFO: usize = 850;
pub const LOG_WARN: usize = 851;
pub const LOG_ERROR: usize = 852;
pub const LOG_DEBUG: usize = 853;

// ---- Threading ----
pub const THREAD_RUN: usize = 870;
pub const THREAD_SLEEP: usize = 871;
