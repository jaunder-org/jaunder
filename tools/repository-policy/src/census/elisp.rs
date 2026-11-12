//! Emacs-reader protocols shared by Elisp census collectors.
//!
//! Each operation reads one source file through Emacs and returns a narrow,
//! collector-owned representation. Process drainage and cleanup remain in
//! `census::process`; this module owns invocation and reader error classification.

use std::io::Write;
use std::process::{Command, Stdio};

use super::process::{self, StderrDrain, StdoutDrain};

pub(crate) enum ReaderError {
    Unavailable,
    Failed(String),
}

impl From<String> for ReaderError {
    fn from(error: String) -> Self {
        Self::Failed(error)
    }
}

pub(crate) fn function_shapes(source: &str) -> Result<Vec<String>, ReaderError> {
    const READER: &str = r#"(progn
(defun census-normalize-tail (value)
  (cond ((consp value) (cons (census-normalize (car value))
                              (census-normalize-tail (cdr value))))
        ((null value) nil)
        (t (census-normalize value))))
(defun census-normalize-binding (binding)
  (if (consp binding)
      (cons 'id (mapcar #'census-normalize (cdr binding)))
    'id))
(defun census-normalize (value &optional head)
  (cond ((and (consp value) (memq (car value) '(let let*)))
         (cons (car value)
               (cons (mapcar #'census-normalize-binding (nth 1 value))
                     (mapcar #'census-normalize (cddr value)))))
        ((and (consp value) (eq (car value) 'lambda))
         (cons 'lambda
               (cons (mapcar (lambda (_argument) 'id) (nth 1 value))
                     (mapcar #'census-normalize (cddr value)))))
        ((consp value) (cons (census-normalize (car value) t)
                             (census-normalize-tail (cdr value))))
        ((symbolp value) (if head value 'id))
        ((numberp value) 'number)
        ((stringp value) 'string)
        (t 'literal)))
(defun census-normalize-definition (form)
  (append (list (car form)
                'id
                (mapcar (lambda (_argument) 'id) (nth 2 form)))
          (mapcar #'census-normalize (cdddr form))))
(with-temp-buffer
  (insert-file-contents "/dev/stdin")
  (emacs-lisp-mode)
  (check-parens)
  (goto-char (point-min))
  (condition-case nil
      (while t
        (let ((form (read (current-buffer))))
          (when (memq (car-safe form) '(defun ert-deftest))
            (princ (prin1-to-string (census-normalize-definition form)))
            (princ "\n"))))
    (end-of-file nil))))"#;
    read(source, READER)
}
pub(crate) fn top_level_dependencies(source: &str) -> Result<Vec<String>, ReaderError> {
    const READER: &str = r#"(with-temp-buffer
  (insert-file-contents "/dev/stdin")
  (emacs-lisp-mode)
  (check-parens)
  (goto-char (point-min))
  (condition-case nil
      (while t
        (let ((form (read (current-buffer))))
          (pcase form
            (`(require (quote ,feature))
             (when (symbolp feature)
               (princ (symbol-name feature))
               (princ "\n"))))))
    (end-of-file nil)))"#;
    read(source, READER)
}

fn read(source: &str, program: &str) -> Result<Vec<String>, ReaderError> {
    let mut reader = Command::new("emacs")
        .args(["--batch", "--quick", "--eval", program])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| {
            if error.kind() == std::io::ErrorKind::NotFound {
                ReaderError::Unavailable
            } else {
                ReaderError::Failed(error.to_string())
            }
        })?;
    let stderr = match reader.stderr.take() {
        Some(stderr) => stderr,
        None => {
            process::terminate_and_reap(&mut reader, "Emacs reader");
            return Err(ReaderError::Failed(
                "Emacs reader stderr was not piped".into(),
            ));
        }
    };
    let mut stderr = StderrDrain::start(stderr);
    let stdout = match reader.stdout.take() {
        Some(stdout) => stdout,
        None => {
            process::terminate_and_reap(&mut reader, "Emacs reader");
            let diagnostics = stderr.finish("Emacs reader");
            return Err(ReaderError::Failed(with_diagnostics(
                "Emacs reader stdout was not piped".into(),
                diagnostics,
            )));
        }
    };
    let mut stdout = StdoutDrain::start(stdout);
    let write_result = match reader.stdin.take() {
        Some(mut stdin) => stdin.write_all(source.as_bytes()),
        None => Err(std::io::Error::other("Emacs reader stdin was not piped")),
    };
    if let Err(error) = write_result {
        process::terminate_and_reap(&mut reader, "Emacs reader");
        finish_stdout_after_failure(&mut stdout);
        let diagnostics = stderr.finish("Emacs reader");
        return Err(ReaderError::Failed(with_diagnostics(
            error.to_string(),
            diagnostics,
        )));
    }
    let status = match reader.wait() {
        Ok(status) => status,
        Err(error) => {
            process::terminate_and_reap(&mut reader, "Emacs reader");
            finish_stdout_after_failure(&mut stdout);
            let diagnostics = stderr.finish("Emacs reader");
            return Err(ReaderError::Failed(with_diagnostics(
                error.to_string(),
                diagnostics,
            )));
        }
    };
    let output = match stdout.finish() {
        Ok(output) => output,
        Err(error) => {
            process::terminate_and_reap(&mut reader, "Emacs reader");
            let diagnostics = stderr.finish("Emacs reader");
            return Err(ReaderError::Failed(with_diagnostics(error, diagnostics)));
        }
    };
    let diagnostics = stderr.finish("Emacs reader");
    if !status.success() {
        return Err(ReaderError::Failed(with_diagnostics(
            format!("Emacs reader exited with {status}"),
            diagnostics,
        )));
    }
    Ok(String::from_utf8_lossy(&output)
        .lines()
        .map(str::to_owned)
        .collect())
}

fn finish_stdout_after_failure(stdout: &mut StdoutDrain) {
    if let Err(error) = stdout.finish() {
        eprintln!("warning: draining census Emacs reader stdout failed: {error}");
    }
}

fn with_diagnostics(error: String, diagnostics: String) -> String {
    if diagnostics.is_empty() {
        error
    } else {
        format!("{error}; Emacs stderr: {diagnostics}")
    }
}
