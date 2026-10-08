"""Regenerate the signed-number fixture (`signed-number-lines.pdf`).

Statistical reports often draw a table as plain text lines, so a line can open with a
negative number (`-0.2 4.5`). The fixture sets such lines -- with a hyphen-minus, a
minus sign (U+2212) and an en dash glued to the number -- next to lines whose dash
really is a list marker (`- 2023.1.12일 ...`, `-외화 ...`, `– 0.3 ...`).

Every test line stands alone: Korean body paragraphs of three tightly spaced lines sit
between them, and the test lines are set farther apart than the body's line spacing,
so each one is a block of its own rather than a continuation of its neighbour.

    python make_signed_number_fixtures.py        # writes next to this script

Requires `reportlab`. Output is deterministic (`invariant=1`).
"""

from pathlib import Path

from reportlab.pdfbase import pdfmetrics
from reportlab.pdfbase.cidfonts import UnicodeCIDFont
from reportlab.pdfgen import canvas

HERE = Path(__file__).resolve().parent

FACE = "HYSMyeongJo-Medium"
SIZE = 12
BODY_LEADING = 15
GAP = 50

PAGES = [
    [
        "-0.2 4.5",
        "-2.9 -4.5 -2.4",
        "−0.8 3.1",
        "–0.7 1.2",
        "-.5",
    ],
    [
        "-12",
        "- 2023.1.12일 실시한 점검 결과를 반영하였다.",
        "-외화 유동성 점검",
        "– 0.3 하락",
    ],
]

BODY = [
    "물가 상승률은 지난해 하반기 이후 둔화 흐름을",
    "이어가고 있으나 목표 수준을 웃도는 상황이며",
    "앞으로의 경로에는 불확실성이 크다고 판단된다",
    "국내 경기는 수출을 중심으로 회복세를 보였고",
    "민간소비는 고금리 영향으로 부진한 모습이다",
    "건설투자는 수주 감소로 조정이 이어질 전망이다",
    "금융시장에서는 장기 금리가 하락하였으며",
    "주가는 반도체 업황 개선 기대로 상승하였다",
    "환율은 미 달러화 강세 영향으로 상승하였다",
    "가계대출은 주택 거래 증가로 확대되었고",
    "기업대출은 운전자금 수요로 늘어났다",
    "은행 예금은 정기예금을 중심으로 증가하였다",
    "통화정책 운용은 물가 안정에 중점을 두되",
    "성장과 금융안정 측면도 함께 고려하였다",
    "기준금리는 현 수준에서 유지하기로 하였다",
    "대외 여건으로는 주요국 통화정책 변화와",
    "지정학적 위험이 계속 영향을 미치고 있다",
    "원자재 가격의 변동성도 여전히 높은 편이다",
    "고용은 서비스업을 중심으로 양호하였으며",
    "실업률은 낮은 수준을 유지하고 있다",
    "임금 상승률은 점차 낮아지는 모습이다",
    "부동산 시장은 지역별로 차별화되고 있으며",
    "수도권 주택가격은 소폭 상승하였다",
    "지방은 미분양 증가로 하락세가 이어졌다",
    "경상수지는 상품수지 흑자로 개선되었고",
    "서비스수지는 여행 수지 적자가 이어졌다",
    "본원소득수지는 배당 수입으로 흑자였다",
    "외환보유액은 전년 말 대비 소폭 늘었다",
    "단기 외채 비중은 안정적인 수준이었다",
    "대외 지급능력에는 문제가 없는 상황이다",
    "향후 물가 경로는 유가와 환율에 달려 있다",
    "농산물 가격의 변동성도 주의할 요인이다",
    "근원물가는 완만한 둔화 흐름이 예상된다",
]


def main() -> None:
    font = UnicodeCIDFont(FACE)
    pdfmetrics.registerFont(font)
    page = canvas.Canvas(str(HERE / "signed-number-lines.pdf"), pagesize=(595, 842), invariant=1)
    body = iter(BODY)
    for tests in PAGES:
        page.setFont(font.fontName, SIZE)
        y = 760

        def paragraph(y: float) -> float:
            for i in range(3):
                page.drawString(72, y, next(body))
                y -= BODY_LEADING
            return y + BODY_LEADING

        for line in tests:
            y = paragraph(y) - GAP
            page.drawString(72, y, line)
            y -= GAP
        paragraph(y)
        page.showPage()
    page.save()


if __name__ == "__main__":
    main()
