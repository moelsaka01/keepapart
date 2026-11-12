# Template: the release workflow renders it with real sha256 values (packaging/render.py)
# and attaches keepapart.rb to the GitHub release, ready for moelsaka01/homebrew-tap.
class Keepapart < Formula
  desc "One machine, many Claude & Codex accounts. Work, personal, clients, kept apart"
  homepage "https://github.com/moelsaka01/keepapart"
  version "{{VERSION}}"
  license "MIT"

  on_macos do
    on_arm do
      url "https://github.com/moelsaka01/keepapart/releases/download/v{{VERSION}}/keepapart-v{{VERSION}}-aarch64-apple-darwin.tar.gz"
      sha256 "{{SHA256:aarch64-apple-darwin}}"
    end
    on_intel do
      url "https://github.com/moelsaka01/keepapart/releases/download/v{{VERSION}}/keepapart-v{{VERSION}}-x86_64-apple-darwin.tar.gz"
      sha256 "{{SHA256:x86_64-apple-darwin}}"
    end
  end

  on_linux do
    on_arm do
      url "https://github.com/moelsaka01/keepapart/releases/download/v{{VERSION}}/keepapart-v{{VERSION}}-aarch64-unknown-linux-musl.tar.gz"
      sha256 "{{SHA256:aarch64-unknown-linux-musl}}"
    end
    on_intel do
      url "https://github.com/moelsaka01/keepapart/releases/download/v{{VERSION}}/keepapart-v{{VERSION}}-x86_64-unknown-linux-musl.tar.gz"
      sha256 "{{SHA256:x86_64-unknown-linux-musl}}"
    end
  end

  def install
    bin.install "keepapart"
  end

  test do
    assert_match version.to_s, shell_output("#{bin}/keepapart --version")
  end
end
