;;; jaunder-delete.el --- Explicit AtomPub Post deletion -*- lexical-binding: t; -*-

;; Copyright (C) 2026 Jaunder contributors

;; This program is free software: you can redistribute it and/or modify
;; it under the terms of the GNU General Public License as published by
;; the Free Software Foundation, either version 3 of the License, or
;; (at your option) any later version.
;;
;; This program is distributed in the hope that it will be useful,
;; but WITHOUT ANY WARRANTY; without even the implied warranty of
;; MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
;; GNU General Public License for more details.
;;
;; You should have received a copy of the GNU General Public License
;; along with this program.  If not, see <https://www.gnu.org/licenses/>.

;;; Commentary:
;; Explicitly delete the remote AtomPub Member represented by a visited Org
;; buffer.  The command validates locally before confirmation or I/O, sends an
;; ETag-guarded DELETE only after confirmation, and removes local state only
;; after the server's unambiguous 204 response.

;;; Code:

(require 'jaunder-config)
(require 'jaunder-org)
(require 'jaunder-transport)

;;;###autoload
(defun jaunder-delete-post ()
  "Conditionally delete the current buffer's AtomPub Post.
The visited Org file must identify a synchronized Post with a numeric
`JAUNDER_ID' and strong `JAUNDER_SYNCED' ETag.  Local state is removed only
after the server answers the confirmed conditional DELETE with HTTP 204."
  (interactive)
  (let ((file (or (buffer-file-name)
                  (error "jaunder: buffer is not visiting a file"))))
    (let* ((id (jaunder--canonical-post-id
                (jaunder--buffer-property "JAUNDER_ID")))
           (etag (jaunder--buffer-property "JAUNDER_SYNCED")))
      (unless id
        (error "jaunder: JAUNDER_ID must be numeric"))
      (unless (jaunder--strong-etag-p etag)
        (error "jaunder: JAUNDER_SYNCED must be a strong ETag"))
      (jaunder--call-with-blog
       file
       (lambda ()
         (when (y-or-n-p (format "Delete Post %s? " id))
           ;; Confirmation is not a lease over an actively edited buffer.
           (when (buffer-modified-p)
             (error "jaunder: refusing to delete a modified visiting buffer"))
           (unless (equal (jaunder--canonical-post-id
                           (jaunder--buffer-property "JAUNDER_ID")) id)
             (error "jaunder: Post identity changed before delete"))
           (let ((response
                  (jaunder--http-request
                   "DELETE" (jaunder--member-url id) nil nil
                   (list (cons "If-Match" etag)))))
             (unless (equal (plist-get response :status) 204)
               (error "jaunder: delete failed (HTTP %s)"
                      (plist-get response :status)))
             ;; The remote 204 is not a lease over a buffer edited while the
             ;; request was in flight.  Preserve local state rather than erase
             ;; the user's post after a remote deletion they must now resolve.
             (when (buffer-modified-p)
               (error "jaunder: remote delete succeeded but local buffer changed"))
             (unless (equal (jaunder--canonical-post-id
                             (jaunder--buffer-property "JAUNDER_ID")) id)
               (error "jaunder: remote delete succeeded but Post identity changed"))
             (delete-file file)
             ;; The only permitted disposition is closing the still-clean buffer.
             (unless (kill-buffer (current-buffer))
               (error "jaunder: deleted file but could not close its buffer")))))))))

(provide 'jaunder-delete)
;;; jaunder-delete.el ends here
