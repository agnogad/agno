#!/usr/bin/env python3
"""Generate a benchmark corpus: N markdown posts with a chosen content profile."""
import argparse
import os
import shutil

WORDS = (
    "static site generator fast build render template content markdown page "
    "layout section taxonomy feed sitemap binary rust parallel speed simple "
    "install config theme draft date slug url html cache incremental"
).split()


def paragraphs(i: int, count: int) -> str:
    out = []
    for p in range(count):
        words = [WORDS[(i * 7 + p * 13 + j) % len(WORDS)] for j in range(60)]
        out.append(" ".join(words).capitalize() + ".")
    return "\n\n".join(out)


def body(i: int, profile: str) -> str:
    if profile == "plain":
        return paragraphs(i, 3) + "\n"

    if profile == "heavy":
        return (
            paragraphs(i, 6)
            + "\n\n## Details\n\n"
            + paragraphs(i + 3, 2)
            + "\n\n```rust\nfn main() {\n    println!(\"bench\");\n}\n```\n"
            + "\n| column | value |\n| --- | --- |\n| a | 1 |\n| b | 2 |\n"
            + "\n```js\nconst x = [1, 2, 3].map((n) => n * 2);\nconsole.log(x);\n```\n"
            + "\nA footnote reference.[^1]\n\n[^1]: A footnote body.\n"
            + "\n```css\n.content { color: #333; }\n```\n"
            + "\n"
            + paragraphs(i + 5, 2)
            + "\n"
        )

    return (
        paragraphs(i, 3)
        + "\n\n```rust\nfn main() {\n    println!(\"bench\");\n}\n```\n"
    )


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("content_dir")
    parser.add_argument("count", type=int)
    parser.add_argument("--profile", default="code", choices=["plain", "code", "heavy"])
    args = parser.parse_args()

    blog = os.path.join(args.content_dir, "blog")
    if os.path.isdir(blog):
        shutil.rmtree(blog)
    os.makedirs(blog, exist_ok=True)

    for i in range(args.count):
        day = (i % 28) + 1
        month = (i % 12) + 1
        front = (
            "---\n"
            f'title: "Benchmark Post {i:04d}"\n'
            f"date: 2024-{month:02d}-{day:02d}\n"
            f'description: "Benchmark post number {i}."\n'
            'tags: ["bench", "post"]\n'
            "---\n\n"
        )
        with open(os.path.join(blog, f"post-{i:04d}.md"), "w", encoding="utf-8") as fh:
            fh.write(front + body(i, args.profile))


if __name__ == "__main__":
    main()
