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
pub const HTTP_SERVER_SERVE_SCRIPT: usize = 703;
pub const WEB_METHOD: usize = 704;
pub const WEB_PATH: usize = 705;
pub const WEB_QUERY: usize = 706;
pub const WEB_FORM: usize = 707;
pub const WEB_HEADER: usize = 708;
pub const WEB_COOKIE: usize = 709;
pub const WEB_BODY: usize = 710;
pub const WEB_SET_STATUS: usize = 711;
pub const WEB_SET_HEADER: usize = 712;
pub const WEB_SET_COOKIE: usize = 713;
pub const WEB_WRITE: usize = 714;
pub const WEB_HTML: usize = 715;
pub const WEB_JSON: usize = 716;
pub const WEB_REDIRECT: usize = 717;
pub const TEMPLATE_RENDER: usize = 718;
pub const WEB_RENDER: usize = 719;
pub const WEB_TEXT: usize = 732;
pub const WEB_FILE: usize = 733;

// ---- Tcp / Udp ----
pub const TCP_CONNECT: usize = 720;
pub const TCP_SEND: usize = 721;
pub const TCP_RECEIVE: usize = 722;
pub const TCP_CLOSE: usize = 723;
pub const UDP_SEND: usize = 730;
pub const UDP_RECEIVE: usize = 731;
pub const SQLITE_OPEN: usize = 740;
pub const SQLITE_EXECUTE: usize = 741;
pub const SQLITE_QUERY: usize = 742;
pub const SQLITE_QUERY_ONE: usize = 743;
pub const SQLITE_LAST_INSERT_ROWID: usize = 744;
pub const SQLITE_CLOSE: usize = 745;
pub const WEB_IS_HTMX: usize = 746;
pub const WEB_SCRIPT_DIR: usize = 747;
pub const WEB_STATIC_DIR: usize = 748;
pub const WEB_PARSE_JSON: usize = 749;

// ---- Task ----
pub const TASK_DELAY: usize = 800;
pub const TASK_RUN: usize = 801;
pub const TASK_WHEN_ALL: usize = 802;
pub const TASK_WHEN_ANY: usize = 803;
pub const TASKGROUP_NEW: usize = 804;
pub const TASKGROUP_RUN: usize = 805;
pub const TASKGROUP_CANCEL: usize = 806;
pub const TASKGROUP_WHEN_ALL: usize = 807;
pub const TASKGROUP_WHEN_ANY: usize = 808;
pub const CTS_NEW: usize = 809;

// ---- GC ----
pub const GC_COLLECT: usize = 820;
pub const GC_STATS: usize = 821;

// ---- Cancellation ----
pub const CTS_CANCEL: usize = 830;
pub const CTS_TOKEN: usize = 831;
pub const TOKEN_IS_CANCELLED: usize = 832;
pub const TOKEN_THROW_IF_CANCELLED: usize = 833;

// ---- Logger ----
pub const LOG_INFO: usize = 850;
pub const LOG_WARN: usize = 851;
pub const LOG_ERROR: usize = 852;
pub const LOG_DEBUG: usize = 853;

// ---- Threading ----
pub const THREAD_RUN: usize = 870;
pub const THREAD_SLEEP: usize = 871;

// ---- Mutex / Channel ----
pub const MUTEX_NEW: usize = 880;
pub const MUTEX_LOCK: usize = 881;
pub const MUTEX_UNLOCK: usize = 882;
pub const MUTEX_TRY_LOCK: usize = 883;
pub const CHANNEL_NEW: usize = 890;
pub const CHANNEL_SEND: usize = 891;
pub const CHANNEL_RECV: usize = 892;
pub const CHANNEL_TRY_RECV: usize = 893;
pub const CHANNEL_CLOSE: usize = 894;

// ---- Generator / Enumerator ----
pub const GEN_FROM: usize = 900;
pub const GEN_RANGE: usize = 901;
pub const GEN_REPEAT: usize = 902;
pub const GEN_EMPTY: usize = 903;
pub const GEN_NEXT: usize = 904;
pub const GEN_HAS_NEXT: usize = 905;
pub const GEN_RESET: usize = 906;
pub const GEN_TO_LIST: usize = 907;

// ---- TimeSpan ----
pub const TIMESPAN_FROM_MS: usize = 920;
pub const TIMESPAN_FROM_SECS: usize = 921;
pub const TIMESPAN_FROM_MINS: usize = 922;
pub const TIMESPAN_FROM_HOURS: usize = 923;
pub const TIMESPAN_ADD: usize = 924;
pub const TIMESPAN_SUB: usize = 925;
pub const TIMESPAN_TOTAL_MS: usize = 926;
pub const TIMESPAN_TOTAL_SECS: usize = 927;
pub const TIMESPAN_TO_STRING: usize = 928;

// ---- DateTime extended ----
pub const DT_ADD_SPAN: usize = 930;
pub const DT_SUB_SPAN: usize = 931;
pub const DT_DIFF: usize = 932;
pub const DT_FORMAT: usize = 933;
pub const DT_PARSE: usize = 934;
pub const DT_YEAR: usize = 935;
pub const DT_MONTH: usize = 936;
pub const DT_DAY: usize = 937;
pub const DT_HOUR: usize = 938;
pub const DT_MINUTE: usize = 939;
pub const DT_SECOND: usize = 940;

// ---- File Streams ----
pub const STREAM_OPEN_READ: usize = 950;
pub const STREAM_OPEN_WRITE: usize = 951;
pub const STREAM_READ: usize = 952;
pub const STREAM_WRITE: usize = 953;
pub const STREAM_CLOSE: usize = 954;
pub const STREAM_SEEK: usize = 955;
pub const STREAM_FLUSH: usize = 956;
pub const STREAM_READ_LINE: usize = 957;
pub const STREAM_WRITE_LINE: usize = 958;

// ---- Compression ----
pub const GZ_COMPRESS: usize = 960;
pub const GZ_DECOMPRESS: usize = 961;
pub const GZ_COMPRESS_FILE: usize = 962;
pub const GZ_DECOMPRESS_FILE: usize = 963;
pub const ZSTD_COMPRESS: usize = 970;
pub const ZSTD_DECOMPRESS: usize = 971;
pub const ZSTD_COMPRESS_FILE: usize = 972;
pub const ZSTD_DECOMPRESS_FILE: usize = 973;

// ---- Math extended ----
pub const MATH_CLAMP: usize = 1000;
pub const MATH_LOG2: usize = 1001;
pub const MATH_LOG10: usize = 1002;
pub const MATH_ATAN2: usize = 1003;
pub const MATH_SIGN: usize = 1004;
pub const MATH_TRUNCATE: usize = 1005;
pub const MATH_IS_NAN: usize = 1006;
pub const MATH_IS_INF: usize = 1007;
pub const MATH_LERP: usize = 1008;
pub const MATH_ASIN: usize = 1009;
pub const MATH_ACOS: usize = 1010;
pub const MATH_ATAN: usize = 1011;
pub const MATH_SINH: usize = 1012;
pub const MATH_COSH: usize = 1013;
pub const MATH_TANH: usize = 1014;
pub const MATH_CBRT: usize = 1015;
pub const MATH_HYPOT: usize = 1016;
pub const MATH_TAU: usize = 1017;

// ---- String extended ----
pub const STR_PAD_LEFT: usize = 1050;
pub const STR_PAD_RIGHT: usize = 1051;
pub const STR_REPEAT: usize = 1052;
pub const STR_REVERSE: usize = 1053;
pub const STR_CHARS: usize = 1054;
pub const STR_LINES: usize = 1055;
pub const STR_PARSE_INT: usize = 1056;
pub const STR_PARSE_FLOAT: usize = 1057;
pub const STR_IS_EMPTY: usize = 1058;
pub const STR_IS_WHITESPACE: usize = 1059;
// STR_JOIN already defined as 212 above
pub const STR_FORMAT: usize = 1061; // static: String.Format(template, args...)
pub const STR_COUNT: usize = 1062;
pub const STR_REMOVE: usize = 1063;
pub const STR_INSERT: usize = 1064;

// ---- List extended ----
pub const LIST_SORT: usize = 1100;
pub const LIST_SORT_DESC: usize = 1101;
pub const LIST_REVERSE: usize = 1102;
pub const LIST_DISTINCT: usize = 1103;
pub const LIST_COUNT: usize = 1104;
pub const LIST_TAKE: usize = 1105;
pub const LIST_SKIP: usize = 1106;
pub const LIST_FLATTEN: usize = 1107;
pub const LIST_ZIP: usize = 1108;
pub const LIST_CHUNK: usize = 1109;
pub const LIST_INDEX_OF: usize = 1110;
pub const LIST_FIND: usize = 1111;
pub const LIST_REDUCE: usize = 1112;
pub const LIST_FILL: usize = 1113;
pub const LIST_COPY: usize = 1114;
pub const LIST_RANGE: usize = 1115;

// ---- Convert static ----
pub const CONV_TO_INT: usize = 1150;
pub const CONV_TO_FLOAT: usize = 1151;
pub const CONV_TO_BOOL: usize = 1152;
pub const CONV_TO_STRING: usize = 1153;
pub const CONV_TO_HEX: usize = 1154;
pub const CONV_FROM_HEX: usize = 1155;
pub const CONV_TO_BYTES: usize = 1156;
pub const CONV_FROM_BYTES: usize = 1157;
pub const CONV_TO_BASE64: usize = 1158;
pub const CONV_FROM_BASE64: usize = 1159;
pub const CONV_TO_BINARY: usize = 1160;

// ---- Env static ----
pub const ENV_GET_VAR: usize = 1200;
pub const ENV_SET_VAR: usize = 1201;
pub const ENV_HAS_VAR: usize = 1202;
pub const ENV_ARGS: usize = 1203;
pub const ENV_CURRENT_DIR: usize = 1204;
pub const ENV_OS: usize = 1205;
pub const ENV_HOME: usize = 1206;
