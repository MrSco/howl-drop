// pairing.hpp: SHA-256 and HMAC-SHA256 against published vectors, keys, and
// who check_head lets in. Transfers end to end are in bridge_test.
#include "pairing.hpp"

#include <cstdio>
#include <set>
#include <string>

namespace {

int failures = 0;

void expect(bool ok, const std::string& what) {
    if (!ok) {
        std::fprintf(stderr, "FAIL: %s\n", what.c_str());
        ++failures;
    }
}

std::string sha_hex(const std::string& text) {
    howldrop::Sha256 sha;
    sha.update(text.data(), text.size());
    return howldrop::to_hex(sha.finish());
}

std::string hmac_hex(const std::string& key, const std::string& text) {
    howldrop::HmacSha256 mac(key);
    mac.update(text);
    return howldrop::to_hex(mac.finish());
}

howldrop::Message file_head(const std::string& name) {
    howldrop::Message m;
    m.type = howldrop::MsgType::File;
    m.body_size = 1234;
    m.fields.set("file", name);
    m.fields.set("from", std::string(32, 'b'));
    return m;
}

}  // namespace

int main() {
    // FIPS 180-2 examples.
    expect(sha_hex("") == "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855", "sha256 of nothing");
    expect(sha_hex("abc") == "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad", "sha256 abc");
    expect(sha_hex("abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq") ==
               "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1",
           "sha256 two blocks");
    {
        howldrop::Sha256 sha;  // a million a's, fed unevenly
        const std::string chunk(999, 'a');
        std::size_t left = 1000000;
        while (left > 0) {
            const std::size_t n = left < chunk.size() ? left : chunk.size();
            sha.update(chunk.data(), n);
            left -= n;
        }
        expect(howldrop::to_hex(sha.finish()) == "cdc76e5c9914fb9281a1c7e284d73e67f1809a48a497200e046d39ccc7112cd0",
               "sha256 a million a's");
    }
    // RFC 4231 test cases 1, 2 and 6 (a key longer than a block).
    expect(hmac_hex(std::string(20, '\x0b'), "Hi There") ==
               "b0344c61d8db38535ca8afceaf0bf12b881dc200c9833da726e9376c2e32cff7",
           "hmac case 1");
    expect(hmac_hex("Jefe", "what do ya want for nothing?") ==
               "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843",
           "hmac case 2");
    expect(hmac_hex(std::string(131, '\xaa'), "Test Using Larger Than Block-Size Key - Hash Key First") ==
               "60e431591ee0b67f0d8a26aacbf5b77f8e0bc6213728c5140546040f0ee37f54",
           "hmac case 6");

    // Keys.
    const std::string key = howldrop::new_pair_key();
    expect(key.size() == 32 && howldrop::normalize_pair_key(key) == key, "a new key is 32 hex digits");
    std::set<std::string> keys;
    for (int i = 0; i < 50; ++i) keys.insert(howldrop::new_pair_key());
    expect(keys.size() == 50, "new keys differ");
    const std::string shown = howldrop::display_pair_key(key);
    expect(shown.size() == 39 && shown[4] == '-', "shown as 8 groups of 4");
    expect(howldrop::normalize_pair_key(" " + shown + "\r\n") == key, "a pasted key with dashes and spaces");
    std::string upper = key;
    for (char& c : upper) c = static_cast<char>(c >= 'a' && c <= 'f' ? c - 'a' + 'A' : c);
    expect(howldrop::normalize_pair_key(upper) == key, "capitals");
    expect(howldrop::normalize_pair_key(key.substr(1)).empty(), "31 digits is not a key");
    expect(howldrop::normalize_pair_key(key + "0").empty(), "33 digits is not a key");
    expect(howldrop::normalize_pair_key("g" + key.substr(1)).empty(), "not hex is not a key");

    // Who check_head lets in.
    const std::string other = howldrop::new_pair_key();
    std::string why;
    {
        auto m = file_head("photo.jpg");
        expect(howldrop::check_head(m, "", why) == howldrop::HeadCheck::Unpaired, "unpaired to unpaired");
        expect(howldrop::check_head(m, key, why) == howldrop::HeadCheck::Refused, "unsigned to a paired PC");
    }
    {
        auto m = file_head("photo.jpg");
        howldrop::PairedSend state;
        expect(howldrop::sign_head(m, key, state), "signs");
        expect(howldrop::check_head(m, key, why) == howldrop::HeadCheck::Paired, "the right key");
        expect(howldrop::check_head(m, other, why) == howldrop::HeadCheck::Refused, "another key");
        expect(howldrop::check_head(m, "", why) == howldrop::HeadCheck::Refused, "signed to an unpaired PC");

        auto renamed = m;
        renamed.fields.set("file", "invoice.exe");
        expect(howldrop::check_head(renamed, key, why) == howldrop::HeadCheck::Refused, "a changed name");
        auto resized = m;
        resized.body_size = 99;
        expect(howldrop::check_head(resized, key, why) == howldrop::HeadCheck::Refused, "a changed size");
        auto retyped = m;
        retyped.type = howldrop::MsgType::Clip;
        expect(howldrop::check_head(retyped, key, why) == howldrop::HeadCheck::Refused, "a changed type");
    }

    // The body MAC depends on both nonces.
    {
        auto a = howldrop::body_mac(key, std::string(32, '1'), std::string(32, '2'));
        auto b = howldrop::body_mac(key, std::string(32, '1'), std::string(32, '3'));
        a.update("same body");
        b.update("same body");
        expect(a.finish() != b.finish(), "a new PC nonce, a new body MAC");
    }

    if (failures == 0) std::printf("pairing_test passed\n");
    return failures == 0 ? 0 : 1;
}
