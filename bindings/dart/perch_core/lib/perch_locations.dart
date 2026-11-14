/// A boundary in the source version used to build a book's search index.
sealed class BookSourceEndpoint {
  const BookSourceEndpoint();

  factory BookSourceEndpoint.fromJson(Map<String, dynamic> json) =>
      switch (json['kind']) {
        'txt' => TxtSourceEndpoint(
          charOffset: json['charOffset'] as int,
          byteOffset: json['byteOffset'] as int,
        ),
        'epub' => EpubSourceEndpoint(
          href: json['href'] as String,
          nodePath: json['nodePath'] as String,
          scalarOffset: json['scalarOffset'] as int,
          utf16Offset: json['utf16Offset'] as int,
        ),
        _ => throw FormatException('Unknown source endpoint: ${json['kind']}'),
      };
}

/// Offsets in the decoded, BOM-stripped TXT used by the reading API.
final class TxtSourceEndpoint extends BookSourceEndpoint {
  const TxtSourceEndpoint({required this.charOffset, required this.byteOffset});

  /// Unicode scalar offset, usable with a matching TXT reading handle's slice.
  final int charOffset;

  /// UTF-8 offset after decoding; GBK/Big5 file-byte offsets differ.
  final int byteOffset;
}

/// A local CFI-style text-node path and two explicitly different offset units.
final class EpubSourceEndpoint extends BookSourceEndpoint {
  const EpubSourceEndpoint({
    required this.href,
    required this.nodePath,
    required this.scalarOffset,
    required this.utf16Offset,
  });

  /// Absolute path within the EPUB container.
  final String href;

  /// Offset-free local text-node path, using the core's XHTML node numbering.
  final String nodePath;

  final int scalarOffset;

  /// Offset in the decoded node's UTF-16 code units, suitable for DOM strings.
  final int utf16Offset;
}

/// Two half-open source endpoints; each EPUB endpoint retains its own href.
final class BookSourceLocation {
  const BookSourceLocation({
    required this.start,
    required this.end,
    required this.containsSynthetic,
  });

  factory BookSourceLocation.fromJson(Map<String, dynamic> json) =>
      BookSourceLocation(
        start: BookSourceEndpoint.fromJson(
          (json['start'] as Map).cast<String, dynamic>(),
        ),
        end: BookSourceEndpoint.fromJson(
          (json['end'] as Map).cast<String, dynamic>(),
        ),
        containsSynthetic: json['containsSynthetic'] as bool,
      );

  final BookSourceEndpoint start, end;

  /// True when the requested indexed range includes artificial separators.
  final bool containsSynthetic;
}
