# Homebrew formula for a tap (moelsaka01/homebrew-tap). Fill in the sha256
# values from the release's .sha256 files after tagging.
class Keepapart < Formula
  desc "One machine, many Claude & Codex accounts. Work, personal, clients, kept apart"
  homepage "https://github.com/moelsaka01/keepapart"
  version "0.1.0"
  license "MIT"

  on_macos do
    on_arm do
      url "https://github.com/moelsaka01/keepapart/releases/download/v0.1.0/keepapart-v0.1.0-aarch64-apple-darwin.tar.gz"
      sha256 "REPLACE_WITH_SHA256_FROM_RELEASE"
    end
    on_intel do
      url "https://github.com/moelsaka01/keepapart/releases/download/v0.1.0/keepapart-v0.1.0-x86_64-apple-darwin.tar.gz"
      sha256 "REPLACE_WITH_SHA256_FROM_RELEASE"
    end
  end

  on_linux do
    on_arm do
      url "https://github.com/moelsaka01/keepapart/releases/download/v0.1.0/keepapart-v0.1.0-aarch64-unknown-linux-musl.tar.gz"
      sha256 "REPLACE_WITH_SHA256_FROM_RELEASE"
    end
    on_intel do
      url "https://github.com/moelsaka01/keepapart/releases/download/v0.1.0/keepapart-v0.1.0-x86_64-unknown-linux-musl.tar.gz"
      sha256 "REPLACE_WITH_SHA256_FROM_RELEASE"
    end
  end

  def install
    bin.install "keepapart"
  end

  test do
    assert_match version.to_s, shell_output("#{bin}/keepapart --version")
  end
end
