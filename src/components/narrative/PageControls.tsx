export function PageControls({
  currentPage,
  lastPage,
  onPage,
}: {
  currentPage: number
  lastPage: number
  onPage: (page: number) => void
}) {
  return (
    <>
      <button className="btn" disabled={currentPage === 0} onClick={() => onPage(currentPage - 1)}>
        Previous page
      </button>
      <span>
        Page {currentPage + 1} of {lastPage + 1}
      </span>
      <button
        className="btn"
        disabled={currentPage === lastPage}
        onClick={() => onPage(currentPage + 1)}
      >
        Next page
      </button>
    </>
  )
}
