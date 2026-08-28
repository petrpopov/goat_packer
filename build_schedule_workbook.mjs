import fs from "node:fs/promises";
import path from "node:path";
import { SpreadsheetFile, Workbook } from "@oai/artifact-tool";

const root = path.resolve("..");
const reportPath = path.resolve("schedule_output/planner_report.json");
const outputDir = path.join(root, "outputs/goat4_daily_scheduler");
const workDir = path.resolve("workbook_work");
const report = JSON.parse(await fs.readFile(reportPath, "utf8"));

const workbook = Workbook.create();
workbook.comments.setSelf({ displayName: "User" });
const dashboard = workbook.worksheets.add("Dashboard");
const inputs = workbook.worksheets.add("Inputs & Research");
const benchmarks = workbook.worksheets.add("Batch Benchmarks");
const schedule = workbook.worksheets.add("Optimal Schedule");
const sensitivity = workbook.worksheets.add("Sensitivity");

const navy = "#17324D";
const blue = "#2F75B5";
const teal = "#0F766E";
const orange = "#ED7D31";
const green = "#70AD47";
const paleBlue = "#EAF3F8";
const paleGreen = "#EAF4E3";
const paleOrange = "#FCECDD";
const paleGray = "#F3F5F7";
const border = "#D5DCE3";

function title(sheet, range, text, subtitle) {
  sheet.getRange(range).merge();
  sheet.getRange(range).values = [[text]];
  sheet.getRange(range).format = {
    fill: navy,
    font: { bold: true, color: "#FFFFFF", size: 18 },
    verticalAlignment: "center",
  };
  const row = Number(range.match(/\d+/)[0]) + 2;
  const endCol = range.split(":")[1].replace(/\d+/g, "");
  const subtitleRange = `A${row}:${endCol}${row}`;
  sheet.getRange(subtitleRange).merge();
  sheet.getRange(subtitleRange).values = [[subtitle]];
  sheet.getRange(subtitleRange).format = {
    fill: "#DCE6F1",
    font: { color: "#44546A", italic: true, size: 10 },
    wrapText: true,
  };
}

function sectionHeader(range, fill = blue) {
  range.format = {
    fill,
    font: { bold: true, color: "#FFFFFF" },
    verticalAlignment: "center",
  };
}

function header(range) {
  range.format = {
    fill: navy,
    font: { bold: true, color: "#FFFFFF" },
    borders: { preset: "inside", style: "thin", color: "#FFFFFF" },
    wrapText: true,
    verticalAlignment: "center",
  };
}

function bodyBorder(range) {
  range.format.borders = {
    insideHorizontal: { style: "thin", color: border },
    bottom: { style: "thin", color: border },
  };
}

function clockLabel(seconds) {
  const day = Math.floor(seconds / 86400);
  const within = seconds % 86400;
  const hh = String(Math.floor(within / 3600)).padStart(2, "0");
  const mm = String(Math.floor((within % 3600) / 60)).padStart(2, "0");
  return day ? `След. день ${hh}:${mm}` : `${hh}:${mm}`;
}

for (const sheet of [dashboard, inputs, benchmarks, schedule, sensitivity]) {
  sheet.showGridLines = false;
}

// Inputs & Research
title(
  inputs,
  "A1:H2",
  "Параметры и источники",
  "Синие ячейки — изменяемые допущения. Для произвольных значений перезапустите Rust-планировщик; таблица sensitivity уже показывает ключевые сценарии."
);
inputs.getRange("A5:D5").values = [["Параметр", "Значение", "Единица", "Комментарий"]];
header(inputs.getRange("A5:D5"));
const inputRows = [
  ["Начало рабочего окна", 10 / 24, "время", "Первый запуск в 10:00"],
  ["Фиксированный ночной запуск", 23 / 24, "время", "Ровно в 23:00; после него до 10:00 запусков нет"],
  ["Конец расчётного цикла", 34 / 24, "[ч]:мм", "10:00 следующего дня"],
  ["Смена стола / перезапуск", report.assumptions.turnaround_minutes, "мин", "Параметр Rust: --turnaround-min"],
  ["Доп. подготовка P1S", 0, "мин", "Уже включена в Bambu total estimated time"],
  ["Подготовка U1", report.assumptions.u1_preparation_minutes, "мин", "Наблюдение пользователя: примерно 10–12 минут"],
  ["Нижняя граница U1", 6, "мин", "Для sensitivity"],
  ["Верхняя граница U1", 15, "мин", "Для sensitivity"],
];
inputs.getRange(`A6:D${5 + inputRows.length}`).values = inputRows;
inputs.getRange("B6:B8").format.numberFormat = "[h]:mm";
inputs.getRange("B9:B13").format.numberFormat = "0";
inputs.getRange("B6:B13").format.fill = "#D9EAF7";
inputs.getRange("B6:B13").format.font = { color: "#1F4E78", bold: true };
bodyBorder(inputs.getRange("A6:D13"));

inputs.getRange("A16:H16").values = [["Исследование времени подготовки U1", null, null, null, null, null, null, null]];
inputs.getRange("A16:H16").merge();
sectionHeader(inputs.getRange("A16:H16"), teal);
inputs.getRange("A17:B21").values = [
  ["Официальный FAQ", report.research.snapmaker_official_faq],
  ["Dynamic Flow Calibration, 1 голова", report.research.official_dynamic_flow_calibration_one_head_minutes / 1440],
  ["Dynamic Flow Calibration, 4 головы", report.research.official_dynamic_flow_calibration_four_heads_minutes / 1440],
  ["Полное per-print preparation", "Официальная единая цифра не опубликована"],
  ["Принято в модели", "11 минут; проверка 6–15 минут на листе Sensitivity"],
];
inputs.getRange("A17:A21").format.font = { bold: true, color: navy };
inputs.getRange("B18:B19").format.numberFormat = "[m]:ss";
inputs.getRange("B17:B21").format.wrapText = true;
bodyBorder(inputs.getRange("A17:B21"));
inputs.getRange("A17:B21").format.rowHeight = 30;
workbook.comments.addThread(
  { cell: inputs.getRange("B17") },
  "Официальный источник Snapmaker: FAQ публикует отдельные времена калибровки, но не полное время подготовки каждой печати."
);

inputs.getRange("A24:H24").values = [["Файлы и воспроизводимость", null, null, null, null, null, null, null]];
inputs.getRange("A24:H24").merge();
sectionHeader(inputs.getRange("A24:H24"), orange);
const sources = report.benchmarks.filter((r, i, a) => a.findIndex(x => x.printer === r.printer) === i);
inputs.getRange("A25:D25").values = [["Принтер", "Исходный 3MF", "SHA-256", "Slicer CLI"]];
header(inputs.getRange("A25:D25"));
inputs.getRange(`A26:D${25 + sources.length}`).values = sources.map(r => [r.printer, r.source_3mf, r.source_sha256, r.slicer]);
inputs.getRange(`B26:D${25 + sources.length}`).format.wrapText = true;
bodyBorder(inputs.getRange(`A26:D${25 + sources.length}`));
inputs.getRange(`A26:D${25 + sources.length}`).format.rowHeight = 48;

inputs.getRange("A:A").format.columnWidth = 31;
inputs.getRange("B:B").format.columnWidth = 52;
inputs.getRange("C:C").format.columnWidth = 16;
inputs.getRange("D:D").format.columnWidth = 56;
inputs.getRange("E:H").format.columnWidth = 12;
inputs.freezePanes.freezeRows(5);

// Batch Benchmarks
title(
  benchmarks,
  "A1:M2",
  "Реальные CLI-бенчмарки партий",
  "Каждая строка — отдельная нарезка подмножества исходного 3MF. P1S total включает стартовую калибровку; для U1 добавлено выбранное время подготовки."
);
benchmarks.getRange("A5:I5").values = [[
  "Принтер",
  "Объектов",
  "Model time",
  "Slicer total",
  "Подготовка",
  "Полный цикл",
  "Пластик, г",
  "Объектов/ч",
  "г/объект",
]];
header(benchmarks.getRange("A5:I5"));
const benchmarkRows = report.benchmarks.map(r => [
  r.printer,
  r.batch_size,
  r.model_seconds / 86400,
  r.slicer_total_seconds / 86400,
  r.preparation_seconds / 86400,
  null,
  r.filament_grams,
  null,
  null,
]);
const bEnd = 5 + benchmarkRows.length;
benchmarks.getRange(`A6:I${bEnd}`).values = benchmarkRows;
for (let row = 6; row <= bEnd; row++) {
  benchmarks.getRange(`F${row}`).formulas = [[`=D${row}+E${row}`]];
  benchmarks.getRange(`H${row}`).formulas = [[`=B${row}/(F${row}*24)`]];
  benchmarks.getRange(`I${row}`).formulas = [[`=G${row}/B${row}`]];
}
benchmarks.getRange(`C6:F${bEnd}`).format.numberFormat = "[h]:mm:ss";
benchmarks.getRange(`G6:G${bEnd}`).format.numberFormat = "0.00";
benchmarks.getRange(`H6:I${bEnd}`).format.numberFormat = "0.00";
bodyBorder(benchmarks.getRange(`A6:I${bEnd}`));
benchmarks.tables.add(`A5:I${bEnd}`, true, "BatchBenchmarksTable").style = "TableStyleMedium2";

benchmarks.getRange("K5:M5").values = [["Размер партии", "P1S объектов/ч", "U1 объектов/ч"]];
header(benchmarks.getRange("K5:M5"));
const p1sRows = report.benchmarks.filter(r => r.printer === "Bambu Lab P1S");
const u1Rows = report.benchmarks.filter(r => r.printer === "Snapmaker U1");
for (let i = 1; i <= 19; i++) {
  const row = 5 + i;
  benchmarks.getRange(`K${row}`).values = [[i]];
  if (i <= p1sRows.length) benchmarks.getRange(`L${row}`).formulas = [[`=H${5 + i}`]];
  if (i <= u1Rows.length) benchmarks.getRange(`M${row}`).formulas = [[`=H${5 + p1sRows.length + i}`]];
}
benchmarks.getRange("L6:M24").format.numberFormat = "0.00";
const efficiencyChart = benchmarks.charts.add("line", benchmarks.getRange("K5:M24"));
efficiencyChart.title = "Производительность растёт с размером партии";
efficiencyChart.hasLegend = true;
efficiencyChart.xAxis = { axisType: "textAxis", title: { text: "Объектов на столе" } };
efficiencyChart.yAxis = { numberFormatCode: "0.0", title: { text: "Объектов в час" } };
efficiencyChart.setPosition("K27", "S44");

benchmarks.getRange("A:A").format.columnWidth = 20;
benchmarks.getRange("B:B").format.columnWidth = 12;
benchmarks.getRange("C:F").format.columnWidth = 15;
benchmarks.getRange("G:I").format.columnWidth = 14;
benchmarks.getRange("K:M").format.columnWidth = 17;
benchmarks.freezePanes.freezeRows(5);

// Optimal Schedule
title(
  schedule,
  "A1:L2",
  "Оптимальное суточное расписание",
  "Условие: дневные задания освобождают принтер к 23:00, затем ровно в 23:00 запускается один последний ночной прогон."
);
schedule.getRange("A5:J5").values = [[
  "Принтер",
  "№",
  "Партия",
  "Старт",
  "Финиш",
  "Полный цикл",
  "Подготовка",
  "Смена после",
  "Накопительно",
  "Ночная",
]];
header(schedule.getRange("A5:J5"));
const jobs = report.printers.flatMap(p => p.variable_batch_schedule.jobs.map(j => ({ ...j, printer: p.printer })));
const p1sSchedule = report.printers[0].variable_batch_schedule.jobs;
const u1Schedule = report.printers[1].variable_batch_schedule.jobs;
const sEnd = 5 + jobs.length;
schedule.getRange(`A6:J${sEnd}`).values = jobs.map((j, index) => [
  j.printer,
  j.sequence,
  j.batch_size,
  j.start_seconds / 86400,
  j.end_seconds / 86400,
  null,
  j.preparation_seconds / 86400,
  j.sequence === report.printers.find(p => p.printer === j.printer).variable_batch_schedule.jobs.length
    ? 0
    : report.assumptions.turnaround_minutes / 1440,
  j.cumulative_objects,
  j.overnight ? "Да" : "Нет",
]);
for (let row = 6; row <= sEnd; row++) schedule.getRange(`F${row}`).formulas = [[`=E${row}-D${row}`]];
schedule.getRange(`D6:H${sEnd}`).format.numberFormat = "[h]:mm";
bodyBorder(schedule.getRange(`A6:J${sEnd}`));
schedule.getRange(`J6:J${sEnd}`).conditionalFormats.add("containsText", {
  text: "Да",
  format: { fill: "#D9EAD3", font: { bold: true, color: "#274E13" } },
});
schedule.tables.add(`A5:J${sEnd}`, true, "OptimalScheduleTable").style = "TableStyleMedium2";

schedule.getRange("L5:Q5").values = [["Принтер", "Оптимум", "Только max", "Прирост", "Заданий", "Завершение"]];
header(schedule.getRange("L5:Q5"));
schedule.getRange("L6:Q7").values = report.printers.map(p => [
  p.printer,
  p.variable_batch_schedule.total_objects,
  p.full_plate_only_schedule.total_objects,
  p.gain_vs_full_plate_only,
  p.variable_batch_schedule.jobs.length,
  p.variable_batch_schedule.completion_seconds / 86400,
]);
schedule.getRange("Q6:Q7").format.numberFormat = "[h]:mm";
schedule.getRange("O6:O7").conditionalFormats.add("cellIs", {
  operator: "greaterThan",
  formula: 0,
  format: { fill: paleGreen, font: { bold: true, color: "#274E13" } },
});
bodyBorder(schedule.getRange("L6:Q7"));

schedule.getRange("L10:N10").values = [["№ запуска", "P1S накопительно", "U1 накопительно"]];
header(schedule.getRange("L10:N10"));
const helperRows = Math.max(p1sSchedule.length, u1Schedule.length);
for (let i = 0; i < helperRows; i++) {
  schedule.getRange(`L${11 + i}:N${11 + i}`).values = [[
    i + 1,
    p1sSchedule[i]?.cumulative_objects ?? null,
    u1Schedule[i]?.cumulative_objects ?? null,
  ]];
}
const cumulativeChart = schedule.charts.add("line", schedule.getRange(`L10:N${10 + helperRows}`));
cumulativeChart.title = "Накопительный выпуск после каждого запуска";
cumulativeChart.hasLegend = true;
cumulativeChart.yAxis = { numberFormatCode: "0", title: { text: "Объектов" } };
cumulativeChart.setPosition("L13", "T30");

schedule.getRange("A:A").format.columnWidth = 20;
schedule.getRange("B:C").format.columnWidth = 10;
schedule.getRange("D:H").format.columnWidth = 14;
schedule.getRange("I:J").format.columnWidth = 13;
schedule.getRange("L:Q").format.columnWidth = 16;
schedule.freezePanes.freezeRows(5);

// Sensitivity
title(
  sensitivity,
  "A1:K2",
  "Чувствительность к подготовке и смене стола",
  "Главный риск — неизвестное полное время подготовки U1. При 10-минутной смене результат меняется с 79 до 76 объектов в диапазоне подготовки 6–15 минут."
);
sensitivity.getRange("A5:G5").values = [[
  "Принтер",
  "Смена, мин",
  "Подготовка, мин",
  "Объектов/сутки",
  "Заданий",
  "Завершение",
  "Партии",
]];
header(sensitivity.getRange("A5:G5"));
const senRows = report.sensitivity.map(r => [
  r.printer,
  r.turnaround_minutes,
  r.preparation_minutes,
  r.total_objects,
  r.jobs,
  r.completion_seconds / 86400,
  r.batch_sequence,
]);
const senEnd = 5 + senRows.length;
sensitivity.getRange(`A6:G${senEnd}`).values = senRows;
sensitivity.getRange(`F6:F${senEnd}`).format.numberFormat = "[h]:mm";
sensitivity.tables.add(`A5:G${senEnd}`, true, "SensitivityRawTable").style = "TableStyleMedium4";

const prepValues = [6, 8, 10, 11, 12, 15];
const turnValues = [0, 5, 10, 15, 20];
sensitivity.getRange("I5:O5").values = [["Смена \\ Подготовка", ...prepValues.map(v => `${v} мин`)]];
header(sensitivity.getRange("I5:O5"));
for (let r = 0; r < turnValues.length; r++) {
  sensitivity.getRange(`I${6 + r}`).values = [[`${turnValues[r]} мин`]];
  const values = prepValues.map(prep => {
    const match = report.sensitivity.find(
      x => x.printer === "Snapmaker U1" && x.turnaround_minutes === turnValues[r] && x.preparation_minutes === prep
    );
    return match?.total_objects ?? null;
  });
  sensitivity.getRange(`J${6 + r}:O${6 + r}`).values = [values];
}
sensitivity.getRange("J6:O10").conditionalFormats.add("colorScale", {
  colors: ["#F8696B", "#FFEB84", "#63BE7B"],
  thresholds: ["min", "50%", "max"],
});
sensitivity.getRange("I6:I10").format.font = { bold: true, color: navy };
bodyBorder(sensitivity.getRange("I6:O10"));

sensitivity.getRange("I14:K14").values = [["Подготовка U1", "Объектов/сутки", "Партии"]];
header(sensitivity.getRange("I14:K14"));
const defaultU1 = report.sensitivity.filter(r => r.printer === "Snapmaker U1" && r.turnaround_minutes === 10);
sensitivity.getRange(`I15:K${14 + defaultU1.length}`).values = defaultU1.map(r => [
  r.preparation_minutes,
  r.total_objects,
  r.batch_sequence,
]);
const prepChart = sensitivity.charts.add("line", sensitivity.getRange(`I14:J${14 + defaultU1.length}`));
prepChart.title = "U1: влияние времени подготовки (смена 10 мин)";
prepChart.hasLegend = false;
prepChart.xAxis = { axisType: "textAxis", title: { text: "Подготовка, мин" } };
prepChart.yAxis = { numberFormatCode: "0", title: { text: "Объектов/сутки" } };
prepChart.setPosition("I23", "Q39");

sensitivity.getRange("A:A").format.columnWidth = 20;
sensitivity.getRange("B:F").format.columnWidth = 15;
sensitivity.getRange("G:G").format.columnWidth = 30;
sensitivity.getRange("I:O").format.columnWidth = 17;
sensitivity.freezePanes.freezeRows(5);

// Dashboard
title(
  dashboard,
  "A1:N2",
  "Максимальный выпуск Goat 4 за сутки",
  "Оптимизация по реальным CLI-временам: дневные задания с 10:00, один фиксированный ночной запуск ровно в 23:00."
);
dashboard.getRange("A5:C5").merge();
dashboard.getRange("D5:F5").merge();
dashboard.getRange("G5:I5").merge();
dashboard.getRange("J5:L5").merge();
dashboard.getRange("A5:C5").values = [["P1S, объектов/сутки"]];
dashboard.getRange("D5:F5").values = [["U1, объектов/сутки"]];
dashboard.getRange("G5:I5").values = [["Вместе"]];
dashboard.getRange("J5:L5").values = [["Прирост против max-пластин"]];
for (const r of ["A5:C5", "D5:F5", "G5:I5", "J5:L5"]) sectionHeader(dashboard.getRange(r), blue);
for (const r of ["A6:C7", "D6:F7", "G6:I7", "J6:L7"]) dashboard.getRange(r).merge();
dashboard.getRange("A6:C7").formulas = [["='Optimal Schedule'!M6"]];
dashboard.getRange("D6:F7").formulas = [["='Optimal Schedule'!M7"]];
dashboard.getRange("G6:I7").formulas = [["='Optimal Schedule'!M6+'Optimal Schedule'!M7"]];
dashboard.getRange("J6:L7").formulas = [["='Optimal Schedule'!O6+'Optimal Schedule'!O7"]];
for (const r of ["A6:C7", "D6:F7", "G6:I7", "J6:L7"]) {
  dashboard.getRange(r).format = {
    fill: paleGreen,
    font: { bold: true, color: "#274E13", size: 22 },
    horizontalAlignment: "center",
    verticalAlignment: "center",
    borders: { preset: "outside", style: "medium", color: green },
  };
}

dashboard.getRange("A10:F10").values = [["P1S: когда запускать", null, null, null, null, null]];
dashboard.getRange("A10:F10").merge();
sectionHeader(dashboard.getRange("A10:F10"), blue);
dashboard.getRange("H10:M10").values = [["Snapmaker U1: когда запускать", null, null, null, null, null]];
dashboard.getRange("H10:M10").merge();
sectionHeader(dashboard.getRange("H10:M10"), teal);
dashboard.getRange("A11:F11").values = [["№", "Старт", "Финиш", "Объектов", "Накопительно", "Режим"]];
dashboard.getRange("H11:M11").values = [["№", "Старт", "Финиш", "Объектов", "Накопительно", "Режим"]];
header(dashboard.getRange("A11:F11"));
header(dashboard.getRange("H11:M11"));
dashboard.getRange(`A12:F${11 + p1sSchedule.length}`).values = p1sSchedule.map(j => [
  j.sequence,
  clockLabel(j.start_seconds),
  clockLabel(j.end_seconds),
  j.batch_size,
  j.cumulative_objects,
  j.overnight ? "Ночная" : "Дневная",
]);
dashboard.getRange(`H12:M${11 + u1Schedule.length}`).values = u1Schedule.map(j => [
  j.sequence,
  clockLabel(j.start_seconds),
  clockLabel(j.end_seconds),
  j.batch_size,
  j.cumulative_objects,
  j.overnight ? "Ночная" : "Дневная",
]);
bodyBorder(dashboard.getRange(`A12:F${11 + p1sSchedule.length}`));
bodyBorder(dashboard.getRange(`H12:M${11 + u1Schedule.length}`));
dashboard.getRange(`F12:F${11 + p1sSchedule.length}`).conditionalFormats.add("containsText", {
  text: "Ночная",
  format: { fill: "#D9E1F2", font: { bold: true, color: navy } },
});
dashboard.getRange(`M12:M${11 + u1Schedule.length}`).conditionalFormats.add("containsText", {
  text: "Ночная",
  format: { fill: "#D9E1F2", font: { bold: true, color: navy } },
});

dashboard.getRange("A20:F20").values = [["Рекомендация", null, null, null, null, null]];
dashboard.getRange("A20:F20").merge();
sectionHeader(dashboard.getRange("A20:F20"), orange);
dashboard.getRange("A21:F24").merge();
dashboard.getRange("A21:F24").values = [[
  "Используйте смешанные партии: P1S 7+16+16+18+18, U1 8+17+17+17+19. Последние полные партии запускаются ровно в 23:00 и завершаются без вашего участия около 02:49/02:54. Это даёт 153 изделия за цикл против 148 при печати только полных столов. Для U1 измерьте фактическую подготовку: при 12 мин выпуск падает до 77, при 15 мин — до 76.",
]];
dashboard.getRange("A21:F24").format = {
  fill: paleOrange,
  font: { color: "#7F4125", size: 11 },
  wrapText: true,
  verticalAlignment: "top",
  borders: { preset: "outside", style: "medium", color: orange },
};

dashboard.getRange("H20:M20").values = [["U1: риск неизвестной подготовки", null, null, null, null, null]];
dashboard.getRange("H20:M20").merge();
sectionHeader(dashboard.getRange("H20:M20"), teal);
dashboard.getRange("H21:J21").values = [["Подг., мин", "Выпуск", "Партии"]];
header(dashboard.getRange("H21:J21"));
dashboard.getRange(`H22:J${21 + defaultU1.length}`).values = defaultU1.map(r => [
  r.preparation_minutes,
  r.total_objects,
  r.batch_sequence,
]);
bodyBorder(dashboard.getRange(`H22:J${21 + defaultU1.length}`));

dashboard.getRange("A:N").format.columnWidth = 14;
dashboard.getRange("A:A").format.columnWidth = 10;
dashboard.getRange("B:C").format.columnWidth = 15;
dashboard.getRange("F:F").format.columnWidth = 13;
dashboard.getRange("H:H").format.columnWidth = 10;
dashboard.getRange("I:J").format.columnWidth = 15;
dashboard.getRange("M:M").format.columnWidth = 13;
dashboard.freezePanes.freezeRows(3);

// Compact inspection and export
const inspect = await workbook.inspect({
  kind: "table",
  range: "Dashboard!A1:N27",
  include: "values,formulas",
  tableMaxRows: 30,
  tableMaxCols: 14,
  maxChars: 8000,
});
console.log(inspect.ndjson);
const errors = await workbook.inspect({
  kind: "match",
  searchTerm: "#REF!|#DIV/0!|#VALUE!|#NAME\\?|#N/A",
  options: { useRegex: true, maxResults: 300 },
  summary: "final formula error scan",
});
console.log(errors.ndjson);

await fs.mkdir(workDir, { recursive: true });
for (const sheetName of ["Dashboard", "Inputs & Research", "Batch Benchmarks", "Optimal Schedule", "Sensitivity"]) {
  const image = await workbook.render({ sheetName, autoCrop: "all", scale: 1, format: "png" });
  const safe = sheetName.toLowerCase().replaceAll(" ", "_").replaceAll("&", "and");
  await fs.writeFile(path.join(workDir, `${safe}.png`), new Uint8Array(await image.arrayBuffer()));
}

await fs.mkdir(outputDir, { recursive: true });
const output = await SpreadsheetFile.exportXlsx(workbook);
const outputPath = path.join(outputDir, "goat4_daily_production_schedule.xlsx");
await output.save(outputPath);
console.log(outputPath);
