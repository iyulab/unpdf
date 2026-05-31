# @iyulab/unpdf

High-performance PDF extraction to Markdown, text, and JSON — WebAssembly build.

## Installation

```bash
npm install @iyulab/unpdf
```

## Usage

### Browser / Bundler (webpack, vite)

```js
import init, { parse, ParseOptions } from '@iyulab/unpdf';

await init();

const response = await fetch('document.pdf');
const bytes = new Uint8Array(await response.arrayBuffer());

const doc = parse(bytes);
console.log(doc.toMarkdown());
console.log(doc.toText());
console.log(`Pages: ${doc.pageCount()}`);
```

### With Options

```js
import init, { parseWithOptions, ParseOptions } from '@iyulab/unpdf';

await init();

const opts = new ParseOptions()
  .lenient()
  .withPassword("secret")
  .withPages(1, 5);

const doc = parseWithOptions(bytes, opts);
console.log(doc.toMarkdown());
```

### Node.js

```js
const { parse } = require('@iyulab/unpdf');
const fs = require('fs');

const bytes = fs.readFileSync('document.pdf');
const doc = parse(new Uint8Array(bytes));
console.log(doc.toText());
```

## API

### Functions

| Function | Signature | Description |
|----------|-----------|-------------|
| `parse` | `(data: Uint8Array) => PdfDocument` | PDF 바이트 파싱 |
| `parseWithOptions` | `(data: Uint8Array, opts: ParseOptions) => PdfDocument` | 옵션 지정 파싱 |

### PdfDocument

| Method | Returns | Description |
|--------|---------|-------------|
| `toMarkdown()` | `string` | Markdown 변환 |
| `toText()` | `string` | Plain text 변환 |
| `toJson()` | `string` | JSON 변환 |
| `pageCount()` | `number` | 총 페이지 수 |
| `metadata()` | `string` | 메타데이터 JSON |

### ParseOptions

| Method | Description |
|--------|-------------|
| `new()` | 기본 옵션 생성 |
| `lenient()` | 오류 무시 모드 |
| `textOnly()` | 텍스트만 추출 |
| `withPassword(pw: string)` | 비밀번호 설정 |
| `withPages(from: number, to: number)` | 페이지 범위 설정 (1-indexed) |

## License

MIT
