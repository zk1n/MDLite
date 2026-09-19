#include "core/Document.h"
#include "editor/EditorAdapter.h"
#include "git/Conflict.h"
#include "assets/Assets.h"
#include "assets/StorageAdapter.h"
#include "calendar/JapaneseHolidays.h"
#include "markdown/Markdown.h"
#include "profiles/Profiles.h"
#include "process/ProcessRunner.h"
#include "search/Search.h"
#include "search/Replace.h"
#include "settings/Settings.h"
#include "table/Table.h"
#include "workspace/Workspace.h"
#include "workspace/Trust.h"

#include <windows.h>
#include <wincodec.h>

#include <array>
#include <filesystem>
#include <fstream>
#include <iostream>
#include <map>
#include <string>
#include <thread>
#include <vector>

namespace {

int failures = 0;

void Check(bool condition, const char* message) {
  if (!condition) {
    std::cerr << "FAIL: " << message << '\n';
    ++failures;
  }
}

bool WriteValidPng(const std::filesystem::path& path) {
  IWICImagingFactory* factory{};
  IWICStream* stream{};
  IWICBitmapEncoder* encoder{};
  IWICBitmapFrameEncode* frame{};
  IPropertyBag2* properties{};
  HRESULT result = CoCreateInstance(CLSID_WICImagingFactory, nullptr, CLSCTX_INPROC_SERVER,
                                    IID_PPV_ARGS(&factory));
  if (SUCCEEDED(result)) result = factory->CreateStream(&stream);
  if (SUCCEEDED(result)) result = stream->InitializeFromFilename(path.c_str(), GENERIC_WRITE);
  if (SUCCEEDED(result)) result = factory->CreateEncoder(GUID_ContainerFormatPng, nullptr, &encoder);
  if (SUCCEEDED(result)) result = encoder->Initialize(stream, WICBitmapEncoderNoCache);
  if (SUCCEEDED(result)) result = encoder->CreateNewFrame(&frame, &properties);
  if (SUCCEEDED(result)) result = frame->Initialize(properties);
  if (SUCCEEDED(result)) result = frame->SetSize(2, 1);
  WICPixelFormatGUID format = GUID_WICPixelFormat32bppBGRA;
  if (SUCCEEDED(result)) result = frame->SetPixelFormat(&format);
  const std::array<unsigned char, 8> pixels{0, 0, 255, 255, 0, 255, 0, 255};
  if (SUCCEEDED(result)) result = frame->WritePixels(1, 8, 8, const_cast<BYTE*>(pixels.data()));
  if (SUCCEEDED(result)) result = frame->Commit();
  if (SUCCEEDED(result)) result = encoder->Commit();
  if (properties) properties->Release();
  if (frame) frame->Release();
  if (encoder) encoder->Release();
  if (stream) stream->Release();
  if (factory) factory->Release();
  if (FAILED(result)) std::cerr << "WIC PNG fixture HRESULT: 0x" << std::hex << static_cast<unsigned long>(result) << std::dec << '\n';
  return SUCCEEDED(result);
}

void WriteBytes(const std::filesystem::path& path, const std::vector<unsigned char>& bytes) {
  std::ofstream output(path, std::ios::binary);
  output.write(reinterpret_cast<const char*>(bytes.data()), static_cast<std::streamsize>(bytes.size()));
}

std::vector<unsigned char> ReadBytes(const std::filesystem::path& path) {
  std::ifstream input(path, std::ios::binary | std::ios::ate);
  const auto size = input.tellg();
  std::vector<unsigned char> bytes(static_cast<std::size_t>(size));
  input.seekg(0);
  input.read(reinterpret_cast<char*>(bytes.data()), size);
  return bytes;
}

void TestUtf8NoOp(const std::filesystem::path& root) {
  const auto path = root / L"utf8.md";
  const std::vector<unsigned char> original{'#', ' ', 'T', 'i', 't', 'l', 'e', '\n', 0xE6, 0x9C, 0xAC, 0xE6, 0x96, 0x87};
  WriteBytes(path, original);
  mdlite::Document document;
  std::wstring error;
  Check(document.Load(path, error), "strict UTF-8 document loads");
  Check(document.encoding() == mdlite::TextEncoding::Utf8, "UTF-8 is detected");
  Check(document.line_ending() == mdlite::LineEnding::Lf, "LF is detected");
  Check(document.Save(error), "unmodified save is a no-op");
  Check(ReadBytes(path) == original, "unmodified save preserves exact bytes");
}

void TestUntitledDocument(const std::filesystem::path& root) {
  mdlite::Document document;
  document.CreateUntitled(root / L".mdlite/.state/untitled/test.md");
  Check(document.untitled() && document.dirty() && document.text().empty(),
        "new untitled document is dirty without creating a normal file");
  std::wstring error;
  Check(!document.Save(error), "untitled document requires an explicit save destination");
  document.MarkEdited(L"draft");
  const auto destination = root / L"saved-untitled.md";
  Check(document.SaveAs(destination, error) && !document.untitled() && !document.dirty(),
        "Save As converts an untitled document into a normal saved document");
  Check(ReadBytes(destination) == std::vector<unsigned char>{'d','r','a','f','t'},
        "untitled Save As writes the exact UTF-8 body");
}

void TestSaveAs(const std::filesystem::path& root) {
  const auto source = root / L"save-as-source.md";
  const auto destination = root / L"save-as-copy.md";
  WriteBytes(source, {'o', 'l', 'd'});
  mdlite::Document document;
  std::wstring error;
  Check(document.Load(source, error), "save-as fixture loads");
  document.MarkEdited(L"new content");
  Check(document.SaveAs(destination, error), "save-as creates a distinct new file");
  Check(document.path() == std::filesystem::absolute(destination).lexically_normal(),
        "save-as updates the document path");
  Check(ReadBytes(source) == std::vector<unsigned char>({'o', 'l', 'd'}),
        "save-as leaves the original file unchanged");
  Check(ReadBytes(destination) ==
            std::vector<unsigned char>({'n', 'e', 'w', ' ', 'c', 'o', 'n', 't', 'e', 'n', 't'}),
        "save-as writes the edited bytes");

  mdlite::Document second;
  Check(second.Load(source, error), "overwrite-refusal fixture loads");
  second.MarkEdited(L"blocked");
  Check(!second.SaveAs(destination, error), "save-as refuses an existing destination");
  Check(ReadBytes(destination) ==
            std::vector<unsigned char>({'n', 'e', 'w', ' ', 'c', 'o', 'n', 't', 'e', 'n', 't'}),
        "save-as refusal does not overwrite the destination");
}

void TestCp932RoundTrip(const std::filesystem::path& root) {
  const auto path = root / L"cp932.md";
  const std::string source = "\x83\x65\x83\x58\x83\x67\r\n";
  WriteBytes(path, std::vector<unsigned char>(source.begin(), source.end()));
  mdlite::Document document;
  std::wstring error;
  Check(document.Load(path, error), "CP932 document loads");
  Check(document.encoding() == mdlite::TextEncoding::Cp932, "CP932 is detected");
  document.MarkEdited(document.text() + L"追記");
  Check(document.Save(error), "CP932 representable edit saves");
  mdlite::Document reloaded;
  Check(reloaded.Load(path, error), "saved CP932 document reloads");
  Check(reloaded.text().ends_with(L"追記"), "CP932 edit round-trips");
  reloaded.MarkEdited(reloaded.text() + L"😀");
  Check(!reloaded.Save(error), "unrepresentable CP932 edit is rejected");
}

void TestExternalConflict(const std::filesystem::path& root) {
  const auto path = root / L"conflict.md";
  WriteBytes(path, {'o', 'l', 'd'});
  mdlite::Document document;
  std::wstring error;
  Check(document.Load(path, error), "conflict fixture loads");
  document.MarkEdited(L"editor");
  Sleep(20);
  WriteBytes(path, {'e', 'x', 't', 'e', 'r', 'n', 'a', 'l'});
  Check(!document.Save(error), "external modification blocks overwrite");
  Check(ReadBytes(path) == std::vector<unsigned char>({'e', 'x', 't', 'e', 'r', 'n', 'a', 'l'}),
        "external bytes remain intact");

  const auto stealth_path = root / L"same-size-time.md";
  WriteBytes(stealth_path, {'o', 'l', 'd'});
  mdlite::Document stealth_document;
  Check(stealth_document.Load(stealth_path, error), "same-size conflict fixture loads");
  const auto original_time = std::filesystem::last_write_time(stealth_path);
  stealth_document.MarkEdited(L"app");
  WriteBytes(stealth_path, {'n', 'e', 'w'});
  std::filesystem::last_write_time(stealth_path, original_time);
  Check(!stealth_document.Save(error), "same-size and restored-time external edit is detected");
  Check(ReadBytes(stealth_path) == std::vector<unsigned char>({'n', 'e', 'w'}),
        "same-size external bytes remain intact");
}

void TestEmptyEncodingAndInvalidBom(const std::filesystem::path& root) {
  std::wstring error;
  const auto utf8_path = root / L"empty-utf8.md";
  WriteBytes(utf8_path, {'t', 'e', 'x', 't'});
  mdlite::Document utf8;
  Check(utf8.Load(utf8_path, error), "UTF-8 empty-save fixture loads");
  utf8.MarkEdited(L"");
  Check(utf8.Save(error), "UTF-8 document can be saved empty");
  Check(ReadBytes(utf8_path).empty(), "empty UTF-8 has zero bytes");

  const auto bom_path = root / L"empty-bom.md";
  WriteBytes(bom_path, {0xEF, 0xBB, 0xBF, 'x'});
  mdlite::Document bom;
  Check(bom.Load(bom_path, error), "UTF-8 BOM empty-save fixture loads");
  bom.MarkEdited(L"");
  Check(bom.Save(error), "UTF-8 BOM document can be saved empty");
  Check(ReadBytes(bom_path) == std::vector<unsigned char>({0xEF, 0xBB, 0xBF}),
        "empty UTF-8 BOM retains only the BOM");

  const auto cp932_path = root / L"empty-cp932.md";
  WriteBytes(cp932_path, {0x83, 0x65, 0x83, 0x58, 0x83, 0x67});
  mdlite::Document cp932;
  Check(cp932.Load(cp932_path, error) && cp932.encoding() == mdlite::TextEncoding::Cp932,
        "CP932 empty-save fixture loads as CP932");
  cp932.MarkEdited(L"");
  Check(cp932.Save(error), "CP932 document can be saved empty");
  Check(ReadBytes(cp932_path).empty(), "empty CP932 has zero bytes");

  const auto invalid_bom_path = root / L"invalid-bom.md";
  WriteBytes(invalid_bom_path, {0xEF, 0xBB, 0xBF, 0x83, 0x65});
  mdlite::Document invalid_bom;
  Check(!invalid_bom.Load(invalid_bom_path, error),
        "invalid UTF-8 after BOM is not silently reclassified as CP932");
}

void TestEditorLineEndingBoundary(const std::filesystem::path& root) {
  const auto lf_path = root / L"lf-edit.md";
  WriteBytes(lf_path, {'a', '\n', 'b', '\n'});
  mdlite::Document lf_document;
  std::wstring error;
  Check(lf_document.Load(lf_path, error), "LF editor fixture loads");
  lf_document.MarkEditedFromEditor(L"a\r\nb changed\r\n");
  Check(lf_document.Save(error), "LF editor edit saves");
  Check(ReadBytes(lf_path) == std::vector<unsigned char>({'a', '\n', 'b', ' ', 'c', 'h', 'a', 'n', 'g', 'e', 'd', '\n'}),
        "RichEdit CRLF expansion does not rewrite an LF document");

  const auto crlf_path = root / L"crlf-edit.md";
  WriteBytes(crlf_path, {'a', '\r', '\n', 'b', '\r', '\n'});
  mdlite::Document crlf_document;
  Check(crlf_document.Load(crlf_path, error), "CRLF editor fixture loads");
  crlf_document.MarkEditedFromEditor(L"a\r\nb changed\r\n");
  Check(crlf_document.Save(error), "CRLF editor edit saves");
  Check(ReadBytes(crlf_path) ==
            std::vector<unsigned char>({'a', '\r', '\n', 'b', ' ', 'c', 'h', 'a', 'n', 'g', 'e', 'd', '\r', '\n'}),
        "CRLF document keeps CRLF after editor edit");

  const auto mixed_path = root / L"mixed-edit.md";
  WriteBytes(mixed_path, {'a', '\r', '\n', 'b', '\n', 'c'});
  mdlite::Document mixed_document;
  Check(mixed_document.Load(mixed_path, error), "mixed-ending editor fixture loads");
  mixed_document.MarkEditedFromEditor(L"a changed\r\nb\r\nc");
  Check(mixed_document.Save(error), "mixed-ending editor edit saves");
  Check(ReadBytes(mixed_path) ==
            std::vector<unsigned char>({'a', ' ', 'c', 'h', 'a', 'n', 'g', 'e', 'd', '\r', '\n', 'b', '\n', 'c'}),
        "existing mixed line-ending sequence survives editor expansion");

  const auto mixed_delete_path = root / L"mixed-delete.md";
  WriteBytes(mixed_delete_path, {'a', '\r', '\n', 'b', '\n', 'c'});
  mdlite::Document mixed_delete;
  Check(mixed_delete.Load(mixed_delete_path, error), "mixed delete fixture loads");
  mixed_delete.MarkEditedFromEditor(L"b\r\nc");
  Check(mixed_delete.Save(error), "mixed document saves after deleting first line");
  Check(ReadBytes(mixed_delete_path) == std::vector<unsigned char>({'b', '\n', 'c'}),
        "deleting a CRLF line does not transfer CRLF to the next LF line");

  const auto mixed_insert_path = root / L"mixed-insert.md";
  WriteBytes(mixed_insert_path, {'a', '\r', '\n', 'b', '\n', 'c'});
  mdlite::Document mixed_insert;
  Check(mixed_insert.Load(mixed_insert_path, error), "mixed insert fixture loads");
  mixed_insert.MarkEditedFromEditor(L"a\r\nnew\r\nb\r\nc");
  Check(mixed_insert.Save(error), "mixed document saves after inserting a line");
  Check(ReadBytes(mixed_insert_path) ==
            std::vector<unsigned char>({'a', '\r', '\n', 'n', 'e', 'w', '\r', '\n', 'b', '\n', 'c'}),
        "insertion keeps unchanged mixed endings attached to their source lines");
}

void TestMarkdown() {
  const std::wstring source = L"---\ntitle: '# metadata'\n---\n# Parent\ntext **bold**\n```\n## code\n```\n## Child\n";
  const auto parsed = mdlite::ParseMarkdown(source);
  Check(parsed.headings.size() == 2, "front matter and fenced headings are excluded");
  Check(parsed.headings[0].level == 1 && parsed.headings[0].text == L"Parent", "H1 is parsed");
  Check(parsed.headings[1].level == 2 && parsed.headings[1].text == L"Child", "H2 is parsed");
  Check(!parsed.spans.empty(), "presentation spans are produced");
  const auto visual = mdlite::ParseMarkdown(L"![代替](assets/a.png)\n| A | B |\n|---|---|\n| 1 | 2 |\n");
  Check(visual.images.size() == 1 && visual.images.front().target == L"assets/a.png",
        "image references retain source ranges and targets");
  Check(visual.tables.size() == 1, "GFM table blocks are identified for native presentation");
  const auto links = mdlite::ParseMarkdown(L"[local](notes/a.md#heading) ![image](a.png) <https://example.test/>\n");
  Check(links.links.size() == 2 && links.links[0].target == L"notes/a.md#heading" &&
            links.links[1].target == L"https://example.test/",
        "Markdown and autolinks are parsed while image syntax stays separate");
  const auto resized = mdlite::ParseMarkdown(
      L"<img src=\"assets/a.png\" alt=\"sample\" width=\"480\">\n");
  Check(resized.images.size() == 1 && resized.images.front().target == L"assets/a.png" &&
            resized.images.front().width_dip == 480,
        "safe generated img markup retains target and display width");
  Check(mdlite::BuildMarkdownEditorSnapshot(
            L"<img src=\"assets/a.png\" alt=\"sample\" width=\"480\">").view == L"\uFFFC",
        "resized img markup remains a native derived image object");

  const auto blocks = mdlite::ParseMarkdown(
      L"paragraph *em* __strong__\n---\n> quote\n- bullet\n1. ordered\n- [x] task\n"
      L"    indented code\n<div>kept</div>\n");
  const auto has_block = [&](mdlite::BlockKind kind) {
    return std::ranges::any_of(blocks.blocks, [kind](const auto& block) { return block.kind == kind; });
  };
  Check(has_block(mdlite::BlockKind::Paragraph) && has_block(mdlite::BlockKind::ThematicBreak) &&
            has_block(mdlite::BlockKind::BlockQuote) && has_block(mdlite::BlockKind::BulletListItem) &&
            has_block(mdlite::BlockKind::OrderedListItem) && has_block(mdlite::BlockKind::TaskListItem) &&
            has_block(mdlite::BlockKind::IndentedCode) && has_block(mdlite::BlockKind::Html),
        "initial CommonMark and GFM block set retains source ranges");
  Check(std::ranges::any_of(blocks.spans, [](const auto& span) {
          return span.kind == mdlite::SpanKind::Emphasis;
        }), "single emphasis is recognized separately from strong emphasis");

  const std::wstring outline = L"# First\nintro\n## Child\nchild\n# Second\nend\n# Third\nlast\n";
  const auto outline_parse = mdlite::ParseMarkdown(outline);
  const auto moved = mdlite::MoveHeadingSection(outline, outline_parse.headings[0].begin,
                                                outline_parse.headings[3].begin);
  Check(moved.changed && moved.text.starts_with(L"# Second"),
        "outline move changes the source order");
  Check(moved.text.find(L"## Child\nchild") > moved.text.find(L"# First\nintro"),
        "outline move keeps child heading content with its parent");
}

void TestEditorAdapter() {
  const auto snapshot = mdlite::BuildEditorSnapshot(L"a\nb😀\r\nc");
  Check(snapshot.view == L"a\r\nb😀\r\nc", "editor view expands only lone LF to CRLF");
  Check(snapshot.inserted_crs.size() == 1 && snapshot.source_size == 8,
        "editor mapping stores one compact index only for each inserted CR");
  Check(snapshot.SourceToView(2) == 3, "source-to-view mapping accounts for expanded LF");
  Check(snapshot.ViewToSource(2) == 1, "inserted CR maps to the source LF boundary");

  const auto edited = mdlite::ApplyEditorText(snapshot, L"a\nb😀\r\nc", L"a\r\nnew\r\nb😀\r\nc");
  Check(edited.changed && edited.source == L"a\nnew\nb😀\r\nc",
        "range transaction preserves LF and existing CRLF without whole-document normalization");
  const auto deleted = mdlite::ApplyEditorText(mdlite::BuildEditorSnapshot(L"a\r\nb\nc"), L"a\r\nb\nc", L"b\r\nc");
  Check(deleted.source == L"b\nc", "range transaction keeps the surviving mixed line ending");
  const auto image = mdlite::BuildMarkdownEditorSnapshot(L"before ![alt](img.png) after");
  Check(image.view == L"before \uFFFC after", "derived editor view represents an image without changing source");
  Check(image.SourceToView(22) == 8 && image.ViewToSource(8) == 22,
        "compact mapping resumes immediately after a collapsed image range");
  const auto image_deleted = mdlite::ApplyEditorText(image, L"before ![alt](img.png) after", L"before  after");
  Check(image_deleted.source == L"before  after", "deleting the derived image removes its complete source range");
  Check(mdlite::ParseMarkdownImages(L"```\n![not-image](code.png)\n```\n![image](real.png)").size() == 1,
        "image-only scan keeps fenced code out of derived image objects");
  const std::wstring image_then_table =
      L"![image](real.png)\n| A | B |\n| --- | --- |\n| 1 | 2 |";
  const auto mixed_snapshot = mdlite::BuildMarkdownEditorSnapshot(image_then_table);
  const auto table_edit = mdlite::InsertTableColumn(
      image_then_table, image_then_table.find(L"1"), true);
  const auto target_snapshot = mdlite::BuildMarkdownEditorSnapshot(table_edit.text);
  const auto mapped_transaction = mdlite::ApplyEditorText(
      mixed_snapshot, image_then_table, target_snapshot.view);
  Check(mapped_transaction.source == table_edit.text,
        "table transaction after a derived image maps back to the exact Markdown source");
}

void TestWorkspaceState(const std::filesystem::path& root) {
  const auto workspace = root / L"workspace";
  std::filesystem::create_directories(workspace);
  mdlite::WorkspaceStore store(workspace);
  std::wstring error;
  Check(store.Initialize(error), "workspace metadata initializes");
  Check(std::filesystem::exists(workspace / L".mdlite/.gitignore"), "workspace gitignore is created");
  Check(std::filesystem::exists(workspace / L".mdlite/templates/memo.md"), "built-in template is created");
  const auto document = workspace / L"note.md";
  WriteBytes(document, {'o', 'l', 'd'});
  std::filesystem::create_directories(workspace / L"nested");
  WriteBytes(workspace / L"Alpha.md", {'a'});
  WriteBytes(workspace / L"nested/project-note.txt", {'b'});
  const auto recent_candidates = mdlite::FindQuickOpenCandidates(
      workspace, L"", {workspace / L"nested/project-note.txt"}, 3);
  Check(!recent_candidates.empty() && recent_candidates.front().filename() == L"project-note.txt",
        "Quick Open ranks recent documents before the remaining Workspace files");
  const auto filtered_candidates = mdlite::FindQuickOpenCandidates(workspace, L"alpha", {}, 10);
  Check(filtered_candidates.size() == 1 && filtered_candidates.front().filename() == L"Alpha.md",
        "Quick Open filters by case-insensitive file name and relative path");
  Check(store.WriteRecovery(document, L"編集中", error), "recovery content writes inside workspace state");
  Check(store.RecoveryFiles().size() == 1, "recovery file is discoverable");
  mdlite::RecoverySnapshot recovery;
  Check(store.ReadRecoverySnapshot(store.RecoveryFiles().front(), recovery, error) &&
            recovery.source_path == document && recovery.text == L"編集中",
        "recovery header is validated and separated from the editable body");
  Check(store.WriteSession({document, L"C:\\outside.md"}, error), "session file writes");
  std::vector<std::filesystem::path> restored;
  Check(store.ReadSession(restored, error), "session file reads");
  Check(restored.size() == 1 && restored.front() == document,
        "session restore includes only existing documents inside the workspace");
  mdlite::SessionState session;
  session.active_index = 0;
  session.main_x = 40;
  session.main_y = 50;
  session.main_width = 1100;
  session.main_height = 700;
  session.recent_documents = {workspace / L"nested/project-note.txt", workspace / L"Alpha.md"};
  session.documents.push_back({document, 1, 2, 3, true, 100, 120, 640, 480});
  Check(store.WriteSessionState(session, error), "detailed session state writes");
  mdlite::SessionState detailed;
  Check(store.ReadSessionState(detailed, error), "detailed session state reads");
  Check(detailed.documents.size() == 1 && detailed.documents[0].selection_begin == 1 &&
            detailed.documents[0].selection_end == 2 && detailed.documents[0].first_visible_line == 3 &&
            detailed.documents[0].compact && detailed.documents[0].width == 640 &&
            detailed.main_width == 1100 && detailed.recent_documents.size() == 2 &&
            detailed.recent_documents[0].filename() == L"project-note.txt",
        "session preserves selection, scroll, compact placement, and main placement");
  Check(store.DiscardRecoverySnapshot(recovery.recovery_path, error),
        "recovery can be explicitly discarded by its validated state path");
  Check(store.RecoveryFiles().empty(), "recovery removal is visible");
}

void TestTrustAndProcess(const std::filesystem::path& root) {
  const auto workspace = root / L"trust";
  std::filesystem::create_directories(workspace / L".mdlite/.state");
  std::wstring error;
  Check(!mdlite::IsWorkspaceTrusted(workspace), "workspace starts untrusted");
  Check(mdlite::SetWorkspaceTrusted(workspace, true, error) && mdlite::IsWorkspaceTrusted(workspace),
        "workspace trust is local and explicit");
  Check(mdlite::SetWorkspaceTrusted(workspace, false, error) && !mdlite::IsWorkspaceTrusted(workspace),
        "workspace trust can be revoked");
  mdlite::ProcessResult process;
  Check(mdlite::RunProcess(L"C:\\Windows\\System32\\cmd.exe", {L"/d", L"/c", L"echo", L"MDLite process"},
                           workspace, 4096, 5000, process, error),
        "process runner starts an argument-array command");
  Check(process.exit_code == 0 && process.output.find(L"MDLite process") != std::wstring::npos,
        "process runner captures bounded output");
  HANDLE cancellation = CreateEventW(nullptr, TRUE, FALSE, nullptr);
  std::thread cancel_thread([&] {
    Sleep(100);
    SetEvent(cancellation);
  });
  error.clear();
  Check(mdlite::RunProcess(L"C:\\Windows\\System32\\ping.exe", {L"-n", L"10", L"127.0.0.1"},
                           workspace, 4096, 10000, process, error, cancellation),
        "process runner accepts an explicit cancellation handle");
  cancel_thread.join();
  CloseHandle(cancellation);
  Check(process.cancelled, "process cancellation terminates its job and reports cancellation");
}

void TestProfiles(const std::filesystem::path& root) {
  const auto workspace = root / L"profiles";
  std::filesystem::create_directories(workspace);
  mdlite::WorkspaceStore store(workspace);
  std::wstring error;
  Check(store.Initialize(error), "profile workspace initializes");
  SYSTEMTIME date{};
  date.wYear = 2026;
  date.wMonth = 9;
  date.wDay = 19;
  const auto built_in_profiles = mdlite::DefaultProfiles();
  mdlite::NoteCreationResult first{};
  Check(mdlite::CreateProfileNote(workspace, built_in_profiles[0], date, first, error),
        "daily note creates");
  Check(first.path == workspace / L"Dairy/2026/202609/20260919.md", "Dairy spelling and path are exact");
  mdlite::NoteCreationResult reopen{};
  Check(mdlite::CreateProfileNote(workspace, built_in_profiles[0], date, reopen, error),
        "existing daily note opens without overwrite");
  Check(!reopen.created && reopen.path == first.path, "daily collision opens existing note");
  mdlite::NoteCreationResult memo1{};
  mdlite::NoteCreationResult memo2{};
  Check(mdlite::CreateProfileNote(workspace, built_in_profiles[2], date, memo1, error),
        "first memo creates");
  Check(mdlite::CreateProfileNote(workspace, built_in_profiles[2], date, memo2, error),
        "second memo creates");
  Check(memo2.path.filename() == L"20260919_01.md", "memo collision uses suffix before extension");

  auto overrides = mdlite::DefaultProfiles();
  overrides[0].directory = L"Journal/{{date:yyyy}}";
  const auto profile_file = workspace / L"profile-overrides.toml";
  Check(mdlite::SaveProfileFile(profile_file, {overrides[0]}, error),
        "profile definitions save atomically");
  std::vector<mdlite::ProfileDefinition> loaded;
  Check(mdlite::LoadProfileFile(profile_file, loaded, error) && loaded.size() == 1,
        "profile definitions round trip through TOML");
  std::vector<mdlite::ProfileDefinition> effective;
  Check(mdlite::ResolveProfiles({}, profile_file, effective, error),
        "profile layer resolves over built-in definitions");
  const auto daily = std::ranges::find_if(effective, [](const auto& item) { return item.id == L"daily"; });
  const auto preview = daily == effective.end() ? std::optional<std::filesystem::path>{} :
      mdlite::PreviewProfilePath(workspace, *daily, date, error);
  Check(preview && *preview == workspace / L"Journal/2026/20260919.md",
        "profile path preview uses the effective editable definition");
  auto unsafe = overrides[0];
  unsafe.directory = L"../outside";
  Check(!mdlite::ValidateProfile(unsafe, error), "profile validation rejects Workspace path escape");
  unsafe = overrides[0];
  unsafe.filename = L"{{unknown}}.md";
  Check(!mdlite::ValidateProfile(unsafe, error), "profile validation rejects undefined variables");
  const auto unsupported_sequence = workspace / L"unsupported-sequence.toml";
  WriteBytes(unsupported_sequence,
             {'s','c','h','e','m','a','_','v','e','r','s','i','o','n',' ','=',' ','1','\n',
              '[','[','p','r','o','f','i','l','e','s',']',']','\n',
              'i','d',' ','=',' ','"','x','"','\n',
              'n','a','m','e',' ','=',' ','"','x','"','\n',
              'd','i','r','e','c','t','o','r','y',' ','=',' ','"','x','"','\n',
              'f','i','l','e','n','a','m','e',' ','=',' ','"','x','.','m','d','"','\n',
              't','e','m','p','l','a','t','e',' ','=',' ','"','t','e','m','p','l','a','t','e','s','/','m','e','m','o','.','m','d','"','\n',
              'c','o','l','l','i','s','i','o','n',' ','=',' ','"','s','e','q','u','e','n','c','e','"','\n',
              's','e','q','u','e','n','c','e','_','f','o','r','m','a','t',' ','=',' ','"','_','%','0','3','d','"','\n'});
  Check(!mdlite::LoadProfileFile(unsupported_sequence, loaded, error),
        "profile parser rejects unsupported sequence format instead of silently changing semantics");

  const std::string input_template = "# {{title}}\n{{input:owner}}\n{{cursor}}end";
  WriteBytes(workspace / L".mdlite/templates/input.md",
             std::vector<unsigned char>(input_template.begin(), input_template.end()));
  mdlite::ProfileDefinition input_profile{
      L"input", L"Input", L"Projects/{{input:owner}}", L"{{title}}.md",
      L"templates/input.md", mdlite::ProfileCollision::OpenExisting,
      {{L"title", L"Title", L"", true}, {L"owner", L"Owner", L"team", false}}};
  const auto input_file = workspace / L"input-profile.toml";
  Check(mdlite::SaveProfileFile(input_file, {input_profile}, error),
        "profile input definitions save");
  Check(mdlite::LoadProfileFile(input_file, loaded, error) && loaded.size() == 1 &&
            loaded.front().inputs.size() == 2 && loaded.front().inputs.front().required,
        "profile input definitions round trip through the shared TOML model");
  mdlite::ProfileValues values{{L"title", L"Roadmap"}, {L"owner", L"alice"}};
  const auto input_preview = mdlite::PreviewProfilePath(workspace, input_profile, date, values, error);
  Check(input_preview && *input_preview == workspace / L"Projects/alice/Roadmap.md",
        "profile path preview expands finite title and input variables");
  mdlite::NoteCreationResult input_note{};
  Check(mdlite::CreateProfileNote(workspace, input_profile, date, values, input_note, error),
        "profile note creates with prompted values");
  const std::vector<unsigned char> expected_input_body{'#',' ','R','o','a','d','m','a','p','\n',
                                                        'a','l','i','c','e','\n','e','n','d'};
  Check(ReadBytes(input_note.path) == expected_input_body && input_note.cursor == 16,
        "profile values expand once and cursor is removed at the expanded UTF-16 position");
  values[L"title"] = L"";
  Check(!mdlite::PreviewProfilePath(workspace, input_profile, date, values, error),
        "profile preview rejects a missing required input");
  values[L"title"] = L"../escape";
  Check(!mdlite::PreviewProfilePath(workspace, input_profile, date, values, error),
        "profile preview rejects path separators introduced by input");
}

void TestSearch(const std::filesystem::path& root) {
  const auto workspace = root / L"search";
  std::filesystem::create_directories(workspace / L"nested");
  WriteBytes(workspace / L"one.md", {'a', 'b', 'c', '\n'});
  WriteBytes(workspace / L"nested/two.md", {'x', 'y', 'z', '\n'});
  std::vector<mdlite::SearchMatch> matches;
  std::wstring error;
  std::map<std::filesystem::path, std::wstring> unsaved{{
      std::filesystem::absolute(workspace / L"one.md").lexically_normal(), L"未保存 abc"}};
  Check(mdlite::SearchWorkspace(workspace, {L"未保存", false, false}, unsaved, matches, error),
        "workspace search includes unsaved text");
  Check(matches.size() == 1 && matches.front().line == 1, "unsaved search result is located");
  Check(mdlite::SearchWorkspace(workspace, {L"x.z", true, true}, {}, matches, error),
        "workspace regex search runs");
  Check(matches.size() == 1, "regex matches disk text");

  WriteBytes(workspace / L"words.md", {'c','a','t',' ','s','c','a','t','t','e','r',' ','c','a','t'});
  mdlite::SearchQuery whole{L"cat", true, false};
  whole.whole_word = true;
  whole.include_globs = {L"*.md"};
  Check(mdlite::SearchWorkspace(workspace, whole, {}, matches, error), "whole-word glob search runs");
  Check(matches.size() == 2, "whole-word search excludes embedded words");

  mdlite::ReplacePlan plan;
  Check(mdlite::PreviewWorkspaceReplace(workspace, whole, L"dog", {}, plan, error),
        "replace preview succeeds");
  Check(plan.files.size() == 1 && plan.files.front().replacement_count == 2,
        "replace preview records exact changes");
  mdlite::ReplaceApplyResult applied;
  Check(mdlite::ApplyWorkspaceReplace(workspace, plan, applied, error), "replace apply succeeds");
  mdlite::Document replaced;
  Check(replaced.Load(workspace / L"words.md", error) && replaced.text() == L"dog scatter dog",
        "replace applies only previewed whole words");
  mdlite::ReplaceApplyResult rolled_back;
  Check(mdlite::RollbackWorkspaceReplace(applied.journal, rolled_back, error),
        "replace journal rollback succeeds");
  Check(replaced.Load(workspace / L"words.md", error) && replaced.text() == L"cat scatter cat",
        "rollback restores the preview baseline");

  error.clear();
  mdlite::ReplacePlan cancelled_preview;
  Check(!mdlite::PreviewWorkspaceReplace(workspace, whole, L"dog", {}, cancelled_preview, error,
                                         [] { return true; }),
        "replace preview can be cancelled");
  Check(error.find(L"中止") != std::wstring::npos, "cancelled preview reports cancellation");

  WriteBytes(workspace / L"cancel-a.md", {'c','a','t'});
  WriteBytes(workspace / L"cancel-b.md", {'c','a','t'});
  mdlite::ReplacePlan cancellable_plan;
  error.clear();
  Check(mdlite::PreviewWorkspaceReplace(workspace, whole, L"dog", {}, cancellable_plan, error),
        "cancellable replace preview succeeds");
  std::size_t cancellation_checks{};
  mdlite::ReplaceApplyResult cancelled_apply;
  Check(!mdlite::ApplyWorkspaceReplace(workspace, cancellable_plan, cancelled_apply, error,
                                       [&] { return ++cancellation_checks > 1; }),
        "replace apply can be cancelled between files");
  Check(cancelled_apply.applied_files == 1 && std::filesystem::exists(cancelled_apply.journal),
        "cancelled replace records the partial apply in a journal");
  mdlite::Document cancel_a;
  mdlite::Document cancel_b;
  Check(cancel_a.Load(workspace / L"cancel-a.md", error) && cancel_a.text() == L"dog" &&
            cancel_b.Load(workspace / L"cancel-b.md", error) && cancel_b.text() == L"cat",
        "cancelled replace changes only the completed file");
  mdlite::ReplaceApplyResult cancelled_rollback;
  Check(mdlite::RollbackWorkspaceReplace(cancelled_apply.journal, cancelled_rollback, error),
        "cancelled replace journal can roll back the partial apply");
}

void TestTableEditing() {
  const std::wstring table = L"| A | B |\n| --- | --- |\n| 1 | 2 |";
  const auto moved = mdlite::MoveToAdjacentTableCell(table, table.find(L"1"), false);
  Check(!moved.changed && moved.selection == table.find(L"2"), "Tab moves to the next table cell");
  const auto appended = mdlite::MoveToAdjacentTableCell(table, table.find(L"2"), false);
  Check(appended.changed && appended.text.ends_with(L"|  |  |"), "Tab at the last cell appends a row");
  const auto inserted = mdlite::InsertTableColumn(table, table.find(L"1"), true);
  Check(inserted.changed && inserted.text.find(L"| --- | --- | --- |") != std::wstring::npos,
        "column insertion extends the delimiter row");
  const auto deleted = mdlite::DeleteTableRow(table, table.find(L"1"));
  Check(deleted.changed && deleted.text.find(L"| 1 | 2 |") == std::wstring::npos,
        "table row deletion removes only the selected row");
  const auto right_inside = mdlite::MoveTableCaretAtBoundary(
      table, table.find(L"1"), mdlite::TableCaretDirection::Right);
  const auto right_boundary = mdlite::MoveTableCaretAtBoundary(
      table, table.find(L"1") + 1, mdlite::TableCaretDirection::Right);
  Check(!right_inside && right_boundary && *right_boundary == table.find(L"2"),
        "Right stays in a cell until its boundary and then moves to the next cell");
  const auto down = mdlite::MoveTableCaretAtBoundary(
      table, table.find(L"A"), mdlite::TableCaretDirection::Down);
  Check(down && *down == table.find(L"1"),
        "Down skips the GFM delimiter row and keeps the table column");
}

void TestJapaneseHolidays() {
  const auto name = mdlite::JapaneseHolidayName(2026, 9, 22);
  Check(name && *name == L"休日", "Cabinet Office holiday data includes 2026-09-22");
  Check(mdlite::JapaneseHolidayYearSupported(2027), "last bundled holiday year is supported");
  Check(!mdlite::JapaneseHolidayYearSupported(2028), "out-of-range holiday year remains unknown");
}

void TestAssets(const std::filesystem::path& root) {
  const auto workspace = root / L"assets";
  const auto source_directory = root / L"asset-source";
  std::filesystem::create_directories(workspace);
  std::filesystem::create_directories(source_directory);
  const auto png = source_directory / L"image.png";
  Check(WriteValidPng(png), "valid PNG fixture is encoded by WIC");
  mdlite::RasterImageInfo image_info;
  std::wstring error;
  Check(mdlite::ReadRasterImageInfo(png, image_info, error) && image_info.width == 2 &&
            image_info.height == 1 && image_info.frame_count == 1,
        "real PNG is decoded with dimensions and frame count");
  IStream* frame_stream{};
  unsigned frame_delay{};
  Check(mdlite::CreateRasterFramePngStream(png, 0, frame_stream, frame_delay, error) &&
            frame_stream != nullptr && frame_delay >= 20,
        "a WIC raster frame is converted to an in-memory PNG for native image refresh");
  if (frame_stream) frame_stream->Release();
  const auto gif = source_directory / L"animated.gif";
  WriteBytes(gif, {
      0x47,0x49,0x46,0x38,0x39,0x61,0x01,0x00,0x01,0x00,0x80,0x00,0x00,
      0x00,0x00,0x00,0xFF,0xFF,0xFF,
      0x21,0xFF,0x0B,0x4E,0x45,0x54,0x53,0x43,0x41,0x50,0x45,0x32,0x2E,0x30,
      0x03,0x01,0x00,0x00,0x00,
      0x21,0xF9,0x04,0x00,0x0A,0x00,0x00,0x00,
      0x2C,0x00,0x00,0x00,0x00,0x01,0x00,0x01,0x00,0x00,0x02,0x02,0x44,0x01,0x00,
      0x21,0xF9,0x04,0x00,0x0A,0x00,0x00,0x00,
      0x2C,0x00,0x00,0x00,0x00,0x01,0x00,0x01,0x00,0x00,0x02,0x02,0x4C,0x01,0x00,
      0x3B});
  error.clear();
  Check(mdlite::ReadRasterImageInfo(gif, image_info, error) && image_info.animated &&
            image_info.frame_count == 2,
        "animated GIF is decoded as a multi-frame raster image");
  frame_stream = nullptr;
  Check(mdlite::CreateRasterFramePngStream(gif, 1, frame_stream, frame_delay, error) &&
            frame_stream != nullptr,
        "a later animated GIF frame is converted for native playback");
  if (frame_stream) frame_stream->Release();
  mdlite::AssetImportResult first{};
  Check(mdlite::ImportImageAsset(png, workspace, workspace / L"note.md", first, error),
        "supported image copies into workspace assets");
  Check(first.relative_reference == L"assets/image.png", "image reference is document-relative");
  Check(WriteValidPng(png), "replacement PNG fixture is valid");
  mdlite::AssetImportResult second{};
  Check(mdlite::ImportImageAsset(png, workspace, workspace / L"note.md", second, error),
        "second image import succeeds without overwrite");
  Check(second.stored_path.filename() == L"image_1.png", "asset collision gets a new name");
  const auto corrupt = source_directory / L"corrupt.png";
  WriteBytes(corrupt, {0x89, 'P', 'N', 'G'});
  mdlite::AssetImportResult corrupt_result{};
  error.clear();
  Check(!mdlite::ImportImageAsset(corrupt, workspace, workspace / L"note.md", corrupt_result, error),
        "corrupt raster image is rejected before a Markdown link is created");
  const auto svg = source_directory / L"unsafe.svg";
  const std::string unsafe = "<svg><script>alert(1)</script></svg>";
  WriteBytes(svg, std::vector<unsigned char>(unsafe.begin(), unsafe.end()));
  mdlite::AssetImportResult svg_result{};
  Check(mdlite::ImportImageAsset(svg, workspace, workspace / L"note.md", svg_result, error),
        "unsafe SVG is preserved as a local asset");
  Check(!svg_result.safe_to_render, "unsafe SVG is disabled for rendering");
  Check(mdlite::ImageHtml(L"a\"b", L"assets/x.png", 480).find(L"width=\"480\"") != std::wstring::npos,
        "image resize markup stores width without height");
}

void TestStorageAdapter(const std::filesystem::path& root) {
  const auto directory = root / L"storage";
  std::filesystem::create_directories(directory);
  const auto asset = directory / L"asset.png";
  WriteBytes(asset, {'P','N','G'});
  const auto config = directory / L"storage.toml";
  WriteBytes(config, std::vector<unsigned char>{
      'e','x','e','c','u','t','a','b','l','e',' ','=',' ','"','C',':','/','W','i','n','d','o','w','s','/','S','y','s','t','e','m','3','2','/','c','m','d','.','e','x','e','"','\n',
      'a','r','g','u','m','e','n','t',' ','=',' ','"','/','d','"','\n',
      'a','r','g','u','m','e','n','t',' ','=',' ','"','/','c','"','\n',
      'a','r','g','u','m','e','n','t',' ','=',' ','"','e','c','h','o','"','\n',
      'a','r','g','u','m','e','n','t',' ','=',' ','"','h','t','t','p','s',':','/','/','e','x','a','m','p','l','e','.','i','n','v','a','l','i','d','/','a','s','s','e','t','"','\n'});
  mdlite::StorageAdapter adapter;
  std::wstring error;
  Check(mdlite::LoadStorageAdapter(config, adapter, error), "storage adapter config loads without credentials");
  mdlite::StorageUploadResult result;
  const bool uploaded = mdlite::UploadWithStorageAdapter(adapter, asset, L"revision-1", nullptr, result, error);
  Check(uploaded,
        "mock storage adapter success is supported");
  Check(result.reference == L"https://example.invalid/asset" && ReadBytes(asset) == std::vector<unsigned char>({'P','N','G'}),
        "storage adapter keeps local asset and returns a reference");

  const auto invalid_config = directory / L"storage-invalid.toml";
  WriteBytes(invalid_config, std::vector<unsigned char>{
      'e','x','e','c','u','t','a','b','l','e',' ','=',' ','"','c','m','d','.','e','x','e','"','\n',
      't','i','m','e','o','u','t','_','m','s',' ','=',' ','n','o','t','-','a','-','n','u','m','b','e','r','\n'});
  error.clear();
  Check(!mdlite::LoadStorageAdapter(invalid_config, adapter, error) && !error.empty(),
        "invalid storage timeout is reported without terminating the app");
}

void TestSettings(const std::filesystem::path& root) {
  const auto directory = root / L"settings";
  const auto common_path = directory / L"common.toml";
  const auto workspace_path = directory / L"workspace.toml";
  mdlite::SettingsLayer common;
  common.theme = mdlite::ThemeMode::Dark;
  common.auto_save = false;
  common.auto_save_delay_ms = 1500;
  common.colors[L"link"] = L"#80A0FF";
  common.font_face = L"Yu Gothic UI";
  common.default_memo_workspace = std::filesystem::absolute(directory / L"memos");
  common.keybindings[L"file.save"] = L"Ctrl+Shift+S";
  std::wstring error;
  Check(mdlite::SaveSettingsLayer(common_path, common, error), "common settings save atomically");
  mdlite::SettingsLayer workspace;
  workspace.theme = mdlite::ThemeMode::Light;
  workspace.font_size_pt = 14;
  Check(mdlite::SaveSettingsLayer(workspace_path, workspace, error), "workspace settings save atomically");
  mdlite::EffectiveSettings effective;
  Check(mdlite::ResolveSettings(common_path, workspace_path, effective, error), "settings hierarchy resolves");
  Check(effective.theme == mdlite::ThemeMode::Light && effective.font_face == L"Yu Gothic UI" &&
            effective.font_size_pt == 14 && !effective.auto_save && effective.auto_save_delay_ms == 1500 &&
            effective.colors[L"link"] == L"#80A0FF" &&
            effective.default_memo_workspace == *common.default_memo_workspace,
        "workspace overrides common while inherited values remain");
  Check(effective.origins[L"theme"] == L"Workspace上書き" &&
            effective.origins[L"font_face"] == L"共通設定",
        "effective settings expose their origins");
  workspace.keybindings[L"file.save"] = L"Ctrl+O";
  Check(mdlite::SaveSettingsLayer(workspace_path, workspace, error), "individual layer accepts a shortcut used only in another layer");
  error.clear();
  Check(!mdlite::ResolveSettings(common_path, workspace_path, effective, error) && !error.empty(),
        "effective keybinding conflicts are rejected");
  mdlite::SettingsLayer invalid;
  invalid.font_face = L"Broken\"Font";
  error.clear();
  Check(!mdlite::SaveSettingsLayer(directory / L"invalid.toml", invalid, error),
        "settings serializer rejects unsafe quoted strings");
  invalid = {};
  invalid.colors[L"unknown"] = L"#FFFFFF";
  error.clear();
  Check(!mdlite::SaveSettingsLayer(directory / L"invalid-color.toml", invalid, error),
        "settings reject unknown custom color keys");
  invalid = {};
  invalid.keybindings[L"file.open"] = L"Ctrl+Shift+NotAKey";
  error.clear();
  Check(!mdlite::SaveSettingsLayer(directory / L"invalid-shortcut.toml", invalid, error),
        "settings reject shortcuts the accelerator parser cannot apply");
  const std::string malformed = "font_size_pt = 12junk\n";
  WriteBytes(directory / L"malformed.toml",
             std::vector<unsigned char>(malformed.begin(), malformed.end()));
  error.clear();
  Check(!mdlite::LoadSettingsLayer(directory / L"malformed.toml", invalid, error),
        "settings reject numeric values with trailing characters");
  const std::string future = "schema_version = 1\n[future]\nnew_option = \"keep-me\"\n";
  const auto future_path = directory / L"future.toml";
  WriteBytes(future_path, std::vector<unsigned char>(future.begin(), future.end()));
  mdlite::SettingsLayer future_layer;
  Check(mdlite::LoadSettingsLayer(future_path, future_layer, error) &&
            mdlite::SaveSettingsLayer(future_path, future_layer, error),
        "settings GUI model can round trip a file with future fields");
  const auto preserved = ReadBytes(future_path);
  const std::string preserved_text(preserved.begin(), preserved.end());
  Check(preserved_text.find("new_option = \"keep-me\"") != std::string::npos,
        "settings save preserves fields unknown to this build");
}

void TestGitConflicts() {
  const std::wstring source =
      L"before\r\n<<<<<<< HEAD\r\ncurrent\r\n||||||| base\r\nold\r\n=======\r\nincoming\r\n>>>>>>> topic\r\nafter\r\n"
      L"<<<<<<< HEAD\nleft\n=======\nright\n>>>>>>> topic\n";
  const auto blocks = mdlite::ParseConflictBlocks(source);
  Check(blocks.size() == 2, "normal and diff3 conflict blocks parse");
  const auto next = mdlite::FindConflictBlock(source, 1, false);
  const auto previous = mdlite::FindConflictBlock(source, 1, true);
  Check(next && *next == 0 && previous && *previous == 1, "conflict navigation wraps in both directions");
  const auto current = mdlite::ResolveConflictBlock(source, 0, mdlite::ConflictChoice::Current);
  Check(current.changed && current.text.find(L"current\r\n") != std::wstring::npos &&
            current.text.find(L"old\r\n") == std::wstring::npos &&
            current.text.find(L"incoming\r\n") == std::wstring::npos,
        "current-side resolution excludes diff3 base and incoming text");
  const auto both = mdlite::ResolveConflictBlock(source, 1, mdlite::ConflictChoice::Both);
  Check(both.changed && both.text.find(L"left\nright\n") != std::wstring::npos,
        "both-side resolution preserves side order and line endings");
  Check(mdlite::ParseConflictBlocks(L"literal <<<<<<< text\n").empty(),
        "inline marker-like text is not a conflict block");
}

}  // namespace

int wmain() {
  const HRESULT com = CoInitializeEx(nullptr, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE);
  const auto root = std::filesystem::temp_directory_path() /
                    (L"mdlite-core-tests-" + std::to_wstring(GetCurrentProcessId()));
  std::error_code error;
  std::filesystem::remove_all(root, error);
  std::filesystem::create_directories(root);
  TestUtf8NoOp(root);
  TestUntitledDocument(root);
  TestSaveAs(root);
  TestCp932RoundTrip(root);
  TestExternalConflict(root);
  TestEmptyEncodingAndInvalidBom(root);
  TestEditorLineEndingBoundary(root);
  TestMarkdown();
  TestEditorAdapter();
  TestWorkspaceState(root);
  TestTrustAndProcess(root);
  TestProfiles(root);
  TestSearch(root);
  TestTableEditing();
  TestJapaneseHolidays();
  TestAssets(root);
  TestStorageAdapter(root);
  TestSettings(root);
  TestGitConflicts();
  std::filesystem::remove_all(root, error);
  if (SUCCEEDED(com)) CoUninitialize();
  if (failures == 0) std::cout << "All MDLite core tests passed.\n";
  return failures == 0 ? 0 : 1;
}
