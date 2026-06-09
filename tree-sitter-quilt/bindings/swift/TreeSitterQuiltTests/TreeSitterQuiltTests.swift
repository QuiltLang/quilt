import XCTest
import SwiftTreeSitter
import TreeSitterQuilt

final class TreeSitterQuiltTests: XCTestCase {
    func testCanLoadGrammar() throws {
        let parser = Parser()
        let language = Language(language: tree_sitter_quilt())
        XCTAssertNoThrow(try parser.setLanguage(language),
                         "Error loading Quilt grammar")
    }
}
