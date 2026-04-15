use pyo3::prelude::*;

#[pyfunction]
fn sum_as_string(left: usize, right: usize) -> String {
    (left + right).to_string()
}

#[pymodule]
fn kaidoku_python(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_function(wrap_pyfunction!(sum_as_string, module)?)?;
    Ok(())
}
