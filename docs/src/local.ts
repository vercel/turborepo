import { ReportRow } from "./config";

/** this will report errors locally, on standard out */
export const reportErrorsLocally = (reportRows: ReportRow[]) => {
  if (reportRows.length === 0) {
    return;
  }
  console.log("This PR introduces broken links to the docs:");
  console.table(reportRows);
};
