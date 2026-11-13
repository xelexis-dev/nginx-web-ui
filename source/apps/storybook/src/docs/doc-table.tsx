import type { ReactNode } from "react";

// Storybook's MDX has no GFM table syntax (remark-gfm is not configured), so docs pages render
// their tables through this component instead of adding a dependency.
export function DocTable({
  columns,
  rows,
  caption,
}: {
  columns: string[];
  rows: ReactNode[][];
  caption?: string;
}) {
  // Columns keep a readable width (min-width in docs.css); on a narrow screen the table scrolls
  // sideways inside this region instead of squeezing every column or widening the page. The region
  // is focusable and named so a keyboard user can scroll it too.
  return (
    <div
      className="nwui-doc-table-scroll"
      role="region"
      aria-label={caption ?? `${columns.join(", ")} 표`}
      tabIndex={0}
    >
      <table className="nwui-doc-table">
        {caption ? <caption>{caption}</caption> : null}
        <thead>
          <tr>
            {columns.map((column) => (
              <th key={column} scope="col">
                {column}
              </th>
            ))}
          </tr>
        </thead>
        <tbody>
          {rows.map((row, index) => (
            <tr key={index}>
              {row.map((cell, cellIndex) => (
                <td key={cellIndex}>{cell}</td>
              ))}
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}
