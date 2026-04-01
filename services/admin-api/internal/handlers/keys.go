package handlers

import (
	"crypto/rand"
	"crypto/sha256"
	"database/sql"
	"fmt"
	"net/http"
	"time"

	"github.com/gin-gonic/gin"
)

type KeysHandler struct {
	pg *sql.DB
}

func NewKeysHandler(pg *sql.DB) *KeysHandler {
	return &KeysHandler{pg: pg}
}

type VirtualKey struct {
	ID          string  `json:"id"`
	Name        string  `json:"name"`
	WorkspaceID string  `json:"workspace_id"`
	CreatedAt   string  `json:"created_at"`
	ExpiresAt   *string `json:"expires_at"`
	Revoked     bool    `json:"revoked"`
}

// GET /admin/v1/keys
func (h *KeysHandler) ListKeys(c *gin.Context) {
	rows, err := h.pg.Query(`
		SELECT id::text, name, workspace_id::text, created_at, expires_at, revoked
		FROM virtual_keys
		ORDER BY created_at DESC
	`)
	if err != nil {
		c.JSON(http.StatusInternalServerError, gin.H{"error": err.Error()})
		return
	}
	defer rows.Close()

	var keys []VirtualKey
	for rows.Next() {
		var k VirtualKey
		var expiresAt sql.NullString
		rows.Scan(&k.ID, &k.Name, &k.WorkspaceID, &k.CreatedAt, &expiresAt, &k.Revoked)
		if expiresAt.Valid {
			k.ExpiresAt = &expiresAt.String
		}
		keys = append(keys, k)
	}
	if keys == nil {
		keys = []VirtualKey{}
	}
	c.JSON(http.StatusOK, keys)
}

// POST /admin/v1/keys
func (h *KeysHandler) CreateKey(c *gin.Context) {
	var body struct {
		Name        string `json:"name" binding:"required"`
		WorkspaceID string `json:"workspace_id" binding:"required"`
	}
	if err := c.ShouldBindJSON(&body); err != nil {
		c.JSON(http.StatusBadRequest, gin.H{"error": err.Error()})
		return
	}

	// Generate key: lgw_sk_<random 32 hex chars>
	raw := make([]byte, 16)
	if _, err := rand.Read(raw); err != nil {
		c.JSON(http.StatusInternalServerError, gin.H{"error": "failed to generate key"})
		return
	}
	token := fmt.Sprintf("lgw_sk_%x", raw)

	// SHA-256 hash it
	h256 := sha256.Sum256([]byte(token))
	keyHash := fmt.Sprintf("%x", h256)

	var id string
	err := h.pg.QueryRow(`
		INSERT INTO virtual_keys (key_hash, workspace_id, name)
		VALUES ($1, $2::uuid, $3)
		RETURNING id::text
	`, keyHash, body.WorkspaceID, body.Name).Scan(&id)

	if err != nil {
		c.JSON(http.StatusInternalServerError, gin.H{"error": err.Error()})
		return
	}

	c.JSON(http.StatusCreated, gin.H{
		"id":           id,
		"token":        token,
		"workspace_id": body.WorkspaceID,
		"name":         body.Name,
		"note":         "Store this token securely. It will not be shown again.",
	})
}

// DELETE /admin/v1/keys/:id
func (h *KeysHandler) RevokeKey(c *gin.Context) {
	id := c.Param("id")
	result, err := h.pg.Exec(`
		UPDATE virtual_keys SET revoked = true WHERE id = $1::uuid
	`, id)
	if err != nil {
		c.JSON(http.StatusInternalServerError, gin.H{"error": err.Error()})
		return
	}
	rows, _ := result.RowsAffected()
	if rows == 0 {
		c.JSON(http.StatusNotFound, gin.H{"error": "key not found"})
		return
	}
	c.JSON(http.StatusOK, gin.H{"status": "revoked", "id": id})
}

// GET /admin/v1/analytics/latency
func (h *KeysHandler) _() {} // spacer

type LatencyHandler struct {
	ch *sql.DB
}

func NewLatencyHandler(ch *sql.DB) *LatencyHandler {
	return &LatencyHandler{ch: ch}
}

// GET /admin/v1/analytics/latency
func (h *LatencyHandler) GetLatency(c *gin.Context) {
	type Bucket struct {
		Bucket   string  `json:"bucket"`
		Count    int64   `json:"count"`
		AvgMs    float64 `json:"avg_ms"`
		P50Ms    float64 `json:"p50_ms"`
		P95Ms    float64 `json:"p95_ms"`
		AvgTtfb  float64 `json:"avg_ttfb_ms"`
		AvgStream float64 `json:"avg_stream_ms"` // latency - ttfb
		Provider string  `json:"provider"`
	}

	rows, err := h.ch.Query(`
		SELECT
			provider_used,
			count(*) AS count,
			avgIf(latency_ms, latency_ms > 0) AS avg_ms,
			quantileIf(0.5)(latency_ms, latency_ms > 0) AS p50,
			quantileIf(0.95)(latency_ms, latency_ms > 0) AS p95,
			avgIf(ttfb_ms, ttfb_ms > 0) AS avg_ttfb
		FROM audit_logs
		GROUP BY provider_used
		ORDER BY avg_ms DESC
	`)
	if err != nil {
		c.JSON(http.StatusInternalServerError, gin.H{"error": err.Error()})
		return
	}
	defer rows.Close()

	var results []Bucket
	for rows.Next() {
		var b Bucket
		rows.Scan(&b.Provider, &b.Count, &b.AvgMs, &b.P50Ms, &b.P95Ms, &b.AvgTtfb)
		b.Bucket = b.Provider
		if b.AvgMs > b.AvgTtfb {
			b.AvgStream = b.AvgMs - b.AvgTtfb
		}
		results = append(results, b)
	}
	if results == nil {
		results = []Bucket{}
	}
	c.JSON(http.StatusOK, results)
}

// GET /admin/v1/analytics/errors
func (h *LatencyHandler) GetErrors(c *gin.Context) {
	_ = time.Now() // import used
	rows, err := h.ch.Query(`
		SELECT
			error_code,
			provider_used,
			count(*) AS count
		FROM audit_logs
		WHERE error_code != ''
		GROUP BY error_code, provider_used
		ORDER BY count DESC
		LIMIT 20
	`)
	if err != nil {
		c.JSON(http.StatusInternalServerError, gin.H{"error": err.Error()})
		return
	}
	defer rows.Close()

	type Row struct {
		ErrorCode string `json:"error_code"`
		Provider  string `json:"provider"`
		Count     int64  `json:"count"`
	}
	var results []Row
	for rows.Next() {
		var r Row
		rows.Scan(&r.ErrorCode, &r.Provider, &r.Count)
		results = append(results, r)
	}
	if results == nil {
		results = []Row{}
	}
	c.JSON(http.StatusOK, results)
}
