#include "tag.h"
#include "tree_sitter/parser.h"

#include <wctype.h>

enum TokenType {
    START_TAG_NAME,
    SCRIPT_START_TAG_NAME,
    STYLE_START_TAG_NAME,
    END_TAG_NAME,
    ERRONEOUS_END_TAG_NAME,
    SELF_CLOSING_TAG_DELIMITER,
    IMPLICIT_END_TAG,
    RAW_TEXT,
    COMMENT,
    TEXT,
};

typedef struct {
    Array(Tag) tags;
} Scanner;

#define MAX(a, b) ((a) > (b) ? (a) : (b))

static inline void advance(TSLexer *lexer) { lexer->advance(lexer, false); }

static inline void skip(TSLexer *lexer) { lexer->advance(lexer, true); }

static unsigned serialize(Scanner *scanner, char *buffer) {
    uint16_t tag_count = scanner->tags.size > UINT16_MAX ? UINT16_MAX : scanner->tags.size;
    uint16_t serialized_tag_count = 0;

    unsigned size = sizeof(tag_count);
    memcpy(&buffer[size], &tag_count, sizeof(tag_count));
    size += sizeof(tag_count);

    for (; serialized_tag_count < tag_count; serialized_tag_count++) {
        Tag tag = scanner->tags.contents[serialized_tag_count];
        if (tag.type == CUSTOM) {
            unsigned name_length = tag.custom_tag_name.size;
            if (name_length > UINT8_MAX) {
                name_length = UINT8_MAX;
            }
            if (size + 2 + name_length >= TREE_SITTER_SERIALIZATION_BUFFER_SIZE) {
                break;
            }
            buffer[size++] = (char)tag.type;
            buffer[size++] = (char)name_length;
            strncpy(&buffer[size], tag.custom_tag_name.contents, name_length);
            size += name_length;
        } else {
            if (size + 1 >= TREE_SITTER_SERIALIZATION_BUFFER_SIZE) {
                break;
            }
            buffer[size++] = (char)tag.type;
        }
    }

    memcpy(&buffer[0], &serialized_tag_count, sizeof(serialized_tag_count));
    return size;
}

static void deserialize(Scanner *scanner, const char *buffer, unsigned length) {
    for (unsigned i = 0; i < scanner->tags.size; i++) {
        tag_free(&scanner->tags.contents[i]);
    }
    array_clear(&scanner->tags);

    if (length > 0) {
        unsigned size = 0;
        uint16_t tag_count = 0;
        uint16_t serialized_tag_count = 0;

        memcpy(&serialized_tag_count, &buffer[size], sizeof(serialized_tag_count));
        size += sizeof(serialized_tag_count);

        memcpy(&tag_count, &buffer[size], sizeof(tag_count));
        size += sizeof(tag_count);

        array_reserve(&scanner->tags, tag_count);
        if (tag_count > 0) {
            unsigned iter = 0;
            for (iter = 0; iter < serialized_tag_count; iter++) {
                Tag tag = tag_new();
                tag.type = (TagType)buffer[size++];
                if (tag.type == CUSTOM) {
                    uint16_t name_length = (uint8_t)buffer[size++];
                    array_reserve(&tag.custom_tag_name, name_length);
                    tag.custom_tag_name.size = name_length;
                    memcpy(tag.custom_tag_name.contents, &buffer[size], name_length);
                    size += name_length;
                }
                array_push(&scanner->tags, tag);
            }
            // add zero tags if we didn't read enough, this is because the
            // buffer had no more room but we held more tags.
            for (; iter < tag_count; iter++) {
                array_push(&scanner->tags, tag_new());
            }
        }
    }
}

static String scan_tag_name(TSLexer *lexer) {
    String tag_name = array_new();
    while (iswalnum(lexer->lookahead) || lexer->lookahead == '-' || lexer->lookahead == ':') {
        array_push(&tag_name, towupper(lexer->lookahead));
        advance(lexer);
    }
    return tag_name;
}

static bool scan_comment(TSLexer *lexer) {
    if (lexer->lookahead != '-') {
        return false;
    }
    advance(lexer);
    if (lexer->lookahead != '-') {
        return false;
    }
    advance(lexer);

    unsigned dashes = 0;
    while (lexer->lookahead) {
        switch (lexer->lookahead) {
            case '-':
                ++dashes;
                break;
            case '>':
                if (dashes >= 2) {
                    lexer->result_symbol = COMMENT;
                    advance(lexer);
                    lexer->mark_end(lexer);
                    return true;
                }
            default:
                dashes = 0;
        }
        advance(lexer);
    }
    return false;
}

static bool scan_raw_text(Scanner *scanner, TSLexer *lexer) {
    if (scanner->tags.size == 0) {
        return false;
    }

    lexer->mark_end(lexer);

    const char *end_delimiter = array_back(&scanner->tags)->type == SCRIPT ? "</SCRIPT" : "</STYLE";
    // Quilt fork: raw text also breaks at hole markers so holes can be
    // interleaved with raw text inside <script>/<style> elements.
    const char *hole_marker = "__QUILT_HOLE__";
    const size_t end_delimiter_len = strlen(end_delimiter);
    const size_t hole_marker_len = strlen(hole_marker);

    // At most one of the two markers can be partially matched at a time (their
    // prefixes share no characters at compatible offsets), so one pair of
    // indices with restart-on-mismatch is enough.
    size_t delimiter_index = 0;
    size_t hole_index = 0;
    bool marked = false; // has any raw text been committed via mark_end?

    while (lexer->lookahead) {
        bool was_matching = delimiter_index > 0 || hole_index > 0;
        bool delimiter_match = towupper(lexer->lookahead) == end_delimiter[delimiter_index];
        bool hole_match = lexer->lookahead == (int32_t)hole_marker[hole_index];

        if (delimiter_match || hole_match) {
            delimiter_index = delimiter_match ? delimiter_index + 1 : 0;
            hole_index = hole_match ? hole_index + 1 : 0;
        } else {
            // Mismatch: the partially-matched marker chars are plain raw text.
            // The current char may still start a fresh marker match.
            delimiter_index = towupper(lexer->lookahead) == end_delimiter[0] ? 1 : 0;
            hole_index = lexer->lookahead == (int32_t)hole_marker[0] ? 1 : 0;
            if (was_matching && (delimiter_index > 0 || hole_index > 0)) {
                // Commit the failed marker's chars before consuming this one.
                lexer->mark_end(lexer);
                marked = true;
            }
        }

        if (delimiter_index == end_delimiter_len) {
            // Stop before `</script` / `</style`; `mark_end` already excludes it.
            break;
        }
        if (hole_index == hole_marker_len) {
            // Stop before the hole. If no raw text precedes it, defer to the
            // internal lexer so it can produce a `quilt_hole` token instead.
            if (!marked) {
                return false;
            }
            break;
        }

        advance(lexer);
        if (delimiter_index == 0 && hole_index == 0) {
            lexer->mark_end(lexer);
            marked = true;
        }
    }

    if (!marked) {
        // Nothing but the end delimiter (or EOF): no raw text to emit.
        return false;
    }
    lexer->result_symbol = RAW_TEXT;
    return true;
}

static void pop_tag(Scanner *scanner) {
    Tag popped_tag = array_pop(&scanner->tags);
    tag_free(&popped_tag);
}

static bool scan_implicit_end_tag(Scanner *scanner, TSLexer *lexer) {
    Tag *parent = scanner->tags.size == 0 ? NULL : array_back(&scanner->tags);

    bool is_closing_tag = false;
    if (lexer->lookahead == '/') {
        is_closing_tag = true;
        advance(lexer);
    } else {
        if (parent && tag_is_void(parent)) {
            pop_tag(scanner);
            lexer->result_symbol = IMPLICIT_END_TAG;
            return true;
        }
    }

    String tag_name = scan_tag_name(lexer);
    if (tag_name.size == 0 && !lexer->eof(lexer)) {
        array_delete(&tag_name);
        return false;
    }

    Tag next_tag = tag_for_name(tag_name);

    if (is_closing_tag) {
        // The tag correctly closes the topmost element on the stack
        if (scanner->tags.size > 0 && tag_eq(array_back(&scanner->tags), &next_tag)) {
            tag_free(&next_tag);
            return false;
        }

        // Otherwise, dig deeper and queue implicit end tags (to be nice in
        // the case of malformed HTML)
        for (unsigned i = scanner->tags.size; i > 0; i--) {
            if (scanner->tags.contents[i - 1].type == next_tag.type) {
                pop_tag(scanner);
                lexer->result_symbol = IMPLICIT_END_TAG;
                tag_free(&next_tag);
                return true;
            }
        }
    } else if (
        parent &&
        (
            !tag_can_contain(parent, &next_tag) ||
            ((parent->type == HTML || parent->type == HEAD || parent->type == BODY) && lexer->eof(lexer))
        )
    ) {
        pop_tag(scanner);
        lexer->result_symbol = IMPLICIT_END_TAG;
        tag_free(&next_tag);
        return true;
    }

    tag_free(&next_tag);
    return false;
}

static bool scan_start_tag_name(Scanner *scanner, TSLexer *lexer) {
    String tag_name = scan_tag_name(lexer);
    if (tag_name.size == 0) {
        array_delete(&tag_name);
        return false;
    }

    Tag tag = tag_for_name(tag_name);
    array_push(&scanner->tags, tag);
    switch (tag.type) {
        case SCRIPT:
            lexer->result_symbol = SCRIPT_START_TAG_NAME;
            break;
        case STYLE:
            lexer->result_symbol = STYLE_START_TAG_NAME;
            break;
        default:
            lexer->result_symbol = START_TAG_NAME;
            break;
    }
    return true;
}

static bool scan_end_tag_name(Scanner *scanner, TSLexer *lexer) {
    String tag_name = scan_tag_name(lexer);

    if (tag_name.size == 0) {
        array_delete(&tag_name);
        return false;
    }

    Tag tag = tag_for_name(tag_name);
    if (scanner->tags.size > 0 && tag_eq(array_back(&scanner->tags), &tag)) {
        pop_tag(scanner);
        lexer->result_symbol = END_TAG_NAME;
    } else {
        lexer->result_symbol = ERRONEOUS_END_TAG_NAME;
    }

    tag_free(&tag);
    return true;
}

static bool scan_self_closing_tag_delimiter(Scanner *scanner, TSLexer *lexer) {
    advance(lexer);
    if (lexer->lookahead == '>') {
        advance(lexer);
        if (scanner->tags.size > 0) {
            pop_tag(scanner);
            lexer->result_symbol = SELF_CLOSING_TAG_DELIMITER;
        }
        return true;
    }
    return false;
}

// Quilt fork: `text` is lexed here rather than by an internal regex so a text
// run can stop before a `__QUILT_HOLE__` marker anywhere inside it (an
// internal maximal-munch token would swallow the marker). Mirrors the
// upstream regex `[^<>&\s]([^<>&]*[^<>&\s])?`: leading whitespace is skipped,
// trailing whitespace is excluded, and `<` `>` `&` (or EOF) end the run.
//
// Like `scan_raw_text`, marker detection restarts its partial-match index on
// mismatch rather than doing a full KMP, so a marker directly preceded by
// extra `_`s (e.g. `text___QUILT_HOLE__`) is missed and lexed as plain text —
// the same corner the marker was always swallowed in before.
static bool scan_text(TSLexer *lexer) {
    const char *hole_marker = "__QUILT_HOLE__";
    const size_t hole_marker_len = strlen(hole_marker);

    while (iswspace(lexer->lookahead)) {
        skip(lexer);
    }
    lexer->mark_end(lexer);

    size_t hole_index = 0;
    bool marked = false; // has any text been committed via mark_end?

    while (lexer->lookahead && lexer->lookahead != '<' && lexer->lookahead != '>' &&
           lexer->lookahead != '&') {
        if (lexer->lookahead == (int32_t)hole_marker[hole_index]) {
            hole_index += 1;
            if (hole_index == hole_marker_len) {
                // Stop before the marker (`mark_end` is at its first char).
                // With no text in front of it, defer to the internal lexer so
                // it can emit a `quilt_hole` token instead.
                break;
            }
        } else {
            if (hole_index > 0) {
                // The failed partial marker match is plain text; commit it
                // before consuming the mismatched char. Marker chars are
                // never whitespace, so this cannot commit a trailing space.
                lexer->mark_end(lexer);
                marked = true;
            }
            hole_index = lexer->lookahead == (int32_t)hole_marker[0] ? 1 : 0;
        }

        int32_t chr = lexer->lookahead;
        advance(lexer);
        if (hole_index == 0 && !iswspace(chr)) {
            lexer->mark_end(lexer);
            marked = true;
        }
    }

    if (hole_index > 0 && hole_index < hole_marker_len) {
        // The run ended (`<` `>` `&` or EOF) mid-partial-match: those marker
        // chars are plain text.
        lexer->mark_end(lexer);
        marked = true;
    }
    if (!marked) {
        return false;
    }
    lexer->result_symbol = TEXT;
    return true;
}

static bool scan(Scanner *scanner, TSLexer *lexer, const bool *valid_symbols) {
    if (valid_symbols[RAW_TEXT] && !valid_symbols[START_TAG_NAME] && !valid_symbols[END_TAG_NAME]) {
        return scan_raw_text(scanner, lexer);
    }

    // Quilt fork: try `text` before the tag logic below; it consumes nothing
    // on the `<` / EOF paths that logic handles (and after a deferred
    // hole-at-start the next char is `_`, which falls through to the
    // do-nothing default arm). `RAW_TEXT` is never valid alongside `TEXT`
    // except during error recovery, which this guard also excludes.
    if (valid_symbols[TEXT] && !valid_symbols[RAW_TEXT] && scan_text(lexer)) {
        return true;
    }

    while (iswspace(lexer->lookahead)) {
        skip(lexer);
    }

    switch (lexer->lookahead) {
        case '<':
            lexer->mark_end(lexer);
            advance(lexer);

            if (lexer->lookahead == '!') {
                advance(lexer);
                return scan_comment(lexer);
            }

            if (valid_symbols[IMPLICIT_END_TAG]) {
                return scan_implicit_end_tag(scanner, lexer);
            }
            break;

        case '\0':
            if (valid_symbols[IMPLICIT_END_TAG]) {
                return scan_implicit_end_tag(scanner, lexer);
            }
            break;

        case '/':
            if (valid_symbols[SELF_CLOSING_TAG_DELIMITER]) {
                return scan_self_closing_tag_delimiter(scanner, lexer);
            }
            break;

        default:
            if ((valid_symbols[START_TAG_NAME] || valid_symbols[END_TAG_NAME]) && !valid_symbols[RAW_TEXT]) {
                return valid_symbols[START_TAG_NAME] ? scan_start_tag_name(scanner, lexer)
                                                     : scan_end_tag_name(scanner, lexer);
            }
    }

    return false;
}

void *tree_sitter_html_external_scanner_create() {
    Scanner *scanner = (Scanner *)ts_calloc(1, sizeof(Scanner));
    return scanner;
}

bool tree_sitter_html_external_scanner_scan(void *payload, TSLexer *lexer, const bool *valid_symbols) {
    Scanner *scanner = (Scanner *)payload;
    return scan(scanner, lexer, valid_symbols);
}

unsigned tree_sitter_html_external_scanner_serialize(void *payload, char *buffer) {
    Scanner *scanner = (Scanner *)payload;
    return serialize(scanner, buffer);
}

void tree_sitter_html_external_scanner_deserialize(void *payload, const char *buffer, unsigned length) {
    Scanner *scanner = (Scanner *)payload;
    deserialize(scanner, buffer, length);
}

void tree_sitter_html_external_scanner_destroy(void *payload) {
    Scanner *scanner = (Scanner *)payload;
    for (unsigned i = 0; i < scanner->tags.size; i++) {
        tag_free(&scanner->tags.contents[i]);
    }
    array_delete(&scanner->tags);
    ts_free(scanner);
}
