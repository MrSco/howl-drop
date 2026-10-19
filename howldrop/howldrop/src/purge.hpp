#pragma once

#include <chrono>
#include <filesystem>

namespace howldrop {

// Delete files under dir that arrived there more than days days before now,
// as the add-on does with save_folder (deliver.py purge): a file's arrival
// is the later of its last write and when it was created here (Windows) or
// last changed (elsewhere), so a years-old photo the phone just sent is new.
// A link is deleted, never followed. Folders left empty go too, except dir
// itself and its text/. Returns how many files were deleted.
int purge_old_files(const std::filesystem::path& dir, int days,
                    std::chrono::system_clock::time_point now = std::chrono::system_clock::now());

}  // namespace howldrop
