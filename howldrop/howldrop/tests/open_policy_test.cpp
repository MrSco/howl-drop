// What the Windows app may open by itself (open_policy.hpp): photos, videos,
// PDFs and text by extension, links only when they are http(s), and nothing
// that runs.
#include "open_policy.hpp"

#include <cstdio>
#include <filesystem>
#include <fstream>
#include <string>

using howldrop::OpenKind;

namespace {

int failures = 0;

void expect(bool ok, const std::string& what) {
    if (!ok) {
        std::fprintf(stderr, "FAIL: %s\n", what.c_str());
        ++failures;
    }
}

std::filesystem::path write(const std::filesystem::path& dir, const std::string& name, const std::string& body) {
    const auto path = dir / name;
    std::ofstream(path, std::ios::binary) << body;
    return path;
}

}  // namespace

int main() {
    expect(howldrop::classify("IMG_4167.JPG") == OpenKind::Image, "JPG is a photo, whatever the case");
    expect(howldrop::classify("photo.heic") == OpenKind::Image, "HEIC is a photo");
    expect(howldrop::classify("IMG_4167.MOV") == OpenKind::Video, "MOV is a video");
    expect(howldrop::classify("notes.pdf") == OpenKind::Document, "PDF is a document");
    expect(howldrop::classify("page.url") == OpenKind::Link, ".url is a link");
    for (const char* name : {"setup.exe", "run.bat", "x.cmd", "x.ps1", "x.vbs", "x.js", "x.msi", "x.lnk",
                             "x.scr", "x.hta", "x.jar", "x.zip", "photo.jpg.exe", "noext"}) {
        expect(howldrop::classify(name) == OpenKind::Other, std::string(name) + " is never opened");
    }

    const auto dir = std::filesystem::temp_directory_path() / "howldrop-open-test";
    std::filesystem::create_directories(dir);
    expect(howldrop::shortcut_url(write(dir, "a.url", "[InternetShortcut]\r\nURL=https://example.com/page\r\n")) ==
               "https://example.com/page",
           "an https shortcut gives its address");
    expect(howldrop::shortcut_url(write(dir, "b.url", "[InternetShortcut]\nURL=HTTP://example.com/\n")) ==
               "HTTP://example.com/",
           "http in any case");
    expect(howldrop::shortcut_url(write(dir, "c.url", "[InternetShortcut]\nURL=file:///C:/Windows/system32/calc.exe\n"))
               .empty(),
           "a file: shortcut gives nothing");
    expect(howldrop::shortcut_url(write(dir, "d.url", "[InternetShortcut]\nURL=\\\\server\\share\\x.exe\n")).empty(),
           "a UNC path gives nothing");
    expect(howldrop::shortcut_url(write(dir, "e.url", "[InternetShortcut]\nURL=javascript:alert(1)\n")).empty(),
           "javascript: gives nothing");
    expect(howldrop::shortcut_url(write(dir, "f.url", "[Other]\nURL=https://example.com/\n")).empty(),
           "URL= outside [InternetShortcut] gives nothing");
    expect(howldrop::shortcut_url(write(dir, "g.txt", "[InternetShortcut]\nURL=https://example.com/\n")).empty(),
           "only a .url is read");
    expect(howldrop::shortcut_url(write(dir, "h.url", "[InternetShortcut]\nURL=https://example.com/\"x\n")).empty(),
           "a quote in the address gives nothing");
    std::filesystem::remove_all(dir);

    if (failures == 0) std::printf("open_policy_test passed\n");
    return failures == 0 ? 0 : 1;
}
