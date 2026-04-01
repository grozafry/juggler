package handlers

import (
	"database/sql"
	"net/http"

	"github.com/gin-gonic/gin"
)

type AnalyticsHandler struct {
	ch *sql.DB
}

func NewAnalyticsHandler(ch *sql.DB) *AnalyticsHandler {
	return &AnalyticsHandler{ch: ch}
}

// GET /admin/v1/analytics/summary
// Global stats across all workspaces
func (h *AnalyticsHandler) GetSummary(c *gin.Context) {
	type Summary struct {
		TotalRequests    int64   `json:"total_requests"`
		TotalPromptTokens int64  `json:"total_prompt_tokens"`
		TotalOutputTokens int64  `json:"total_output_tokens"`
		TotalTokens      int64   `json:"total_tokens"`
		TotalCostUSD     float64 `json:"total_cost_usd"`
		AvgLatencyMs     float64 `json:"avg_latency_ms"`
		AvgTtfbMs        float64 `json:"avg_ttfb_ms"`
	}

	var s Summary
	err := h.ch.QueryRow(`
		SELECT count(*),
		       sum(prompt_token_count),
		       sum(completion_token_count),
		       sum(prompt_token_count + completion_token_count),
		       sumIf(cost_usd, cost_usd > 0),
		       avgIf(latency_ms, latency_ms > 0),
		       avgIf(ttfb_ms, ttfb_ms > 0)
		FROM audit_logs
	`).Scan(&s.TotalRequests, &s.TotalPromptTokens, &s.TotalOutputTokens, &s.TotalTokens, &s.TotalCostUSD, &s.AvgLatencyMs, &s.AvgTtfbMs)
	if err != nil && err != sql.ErrNoRows {
		c.JSON(http.StatusInternalServerError, gin.H{"error": err.Error()})
		return
	}
	c.JSON(http.StatusOK, s)
}

// GET /admin/v1/analytics/timeline?hours=24
func (h *AnalyticsHandler) GetTimeline(c *gin.Context) {
	hours := c.DefaultQuery("hours", "24")
	rows, err := h.ch.Query(`
		SELECT 
			toStartOfHour(timestamp) AS hour,
			count(*) AS requests,
			sum(prompt_token_count + completion_token_count) AS tokens
		FROM audit_logs
		WHERE timestamp >= now() - INTERVAL ? HOUR
		GROUP BY hour
		ORDER BY hour ASC
	`, hours)
	if err != nil {
		c.JSON(http.StatusInternalServerError, gin.H{"error": err.Error()})
		return
	}
	defer rows.Close()

	type Point struct {
		Hour     string `json:"hour"`
		Requests int64  `json:"requests"`
		Tokens   int64  `json:"tokens"`
	}
	var points []Point
	for rows.Next() {
		var p Point
		rows.Scan(&p.Hour, &p.Requests, &p.Tokens)
		points = append(points, p)
	}
	if points == nil {
		points = []Point{}
	}
	c.JSON(http.StatusOK, points)
}

// GET /admin/v1/analytics/providers
func (h *AnalyticsHandler) GetProviders(c *gin.Context) {
	rows, err := h.ch.Query(`
		SELECT 
			provider_used,
			count(*) AS requests,
			sum(prompt_token_count + completion_token_count) AS tokens,
			sumIf(cost_usd, not isNaN(cost_usd)) AS cost_usd
		FROM audit_logs
		GROUP BY provider_used
		ORDER BY requests DESC
	`)
	if err != nil {
		c.JSON(http.StatusInternalServerError, gin.H{"error": err.Error()})
		return
	}
	defer rows.Close()

	type Row struct {
		Provider string  `json:"provider"`
		Requests int64   `json:"requests"`
		Tokens   int64   `json:"tokens"`
		CostUSD  float64 `json:"cost_usd"`
	}
	var results []Row
	for rows.Next() {
		var r Row
		rows.Scan(&r.Provider, &r.Requests, &r.Tokens, &r.CostUSD)
		results = append(results, r)
	}
	if results == nil {
		results = []Row{}
	}
	c.JSON(http.StatusOK, results)
}

// GET /admin/v1/analytics/models
func (h *AnalyticsHandler) GetModels(c *gin.Context) {
	rows, err := h.ch.Query(`
		SELECT 
			model,
			provider_used,
			count(*) AS requests
		FROM audit_logs
		GROUP BY model, provider_used
		ORDER BY requests DESC
		LIMIT 20
	`)
	if err != nil {
		c.JSON(http.StatusInternalServerError, gin.H{"error": err.Error()})
		return
	}
	defer rows.Close()

	type Row struct {
		Model    string `json:"model"`
		Provider string `json:"provider"`
		Requests int64  `json:"requests"`
	}
	var results []Row
	for rows.Next() {
		var r Row
		rows.Scan(&r.Model, &r.Provider, &r.Requests)
		results = append(results, r)
	}
	if results == nil {
		results = []Row{}
	}
	c.JSON(http.StatusOK, results)
}

// GET /admin/v1/analytics/recent
func (h *AnalyticsHandler) GetRecentLogs(c *gin.Context) {
	rows, err := h.ch.Query(`
		SELECT request_id, timestamp, workspace_id, model, provider_used,
		       latency_ms, ttfb_ms,
		       prompt_token_count, completion_token_count,
		       cost_usd, error_code
		FROM audit_logs
		ORDER BY timestamp DESC
		LIMIT 50
	`)
	if err != nil {
		c.JSON(http.StatusInternalServerError, gin.H{"error": err.Error()})
		return
	}
	defer rows.Close()

	type Row struct {
		RequestID       string  `json:"request_id"`
		Timestamp       string  `json:"timestamp"`
		WorkspaceID     string  `json:"workspace_id"`
		Model           string  `json:"model"`
		Provider        string  `json:"provider"`
		LatencyMs       uint32  `json:"latency_ms"`
		TtfbMs          uint32  `json:"ttfb_ms"`
		PromptTokens    uint32  `json:"prompt_token_count"`
		CompletionTokens uint32 `json:"completion_token_count"`
		CostUSD         float64 `json:"cost_usd"`
		ErrorCode       string  `json:"error_code"`
	}
	var results []Row
	for rows.Next() {
		var r Row
		rows.Scan(&r.RequestID, &r.Timestamp, &r.WorkspaceID, &r.Model, &r.Provider,
			&r.LatencyMs, &r.TtfbMs, &r.PromptTokens, &r.CompletionTokens, &r.CostUSD, &r.ErrorCode)
		results = append(results, r)
	}
	if results == nil {
		results = []Row{}
	}
	c.JSON(http.StatusOK, results)
}

// GET /admin/v1/analytics/costs (kept for backward compat)
func (h *AnalyticsHandler) GetCosts(c *gin.Context) {
	workspaceID := c.Query("workspace_id")
	if workspaceID == "" {
		c.JSON(http.StatusBadRequest, gin.H{"error": "workspace_id query parameter is required"})
		return
	}

	var totalCost float64
	var totalTokens int64
	var requestCount int64

	err := h.ch.QueryRow(`
		SELECT 
			sumIf(cost_usd, not isNaN(cost_usd)), 
			sum(prompt_token_count + completion_token_count),
			count(*)
		FROM audit_logs
		WHERE workspace_id = ?
	`, workspaceID).Scan(&totalCost, &totalTokens, &requestCount)

	if err != nil && err != sql.ErrNoRows {
		c.JSON(http.StatusInternalServerError, gin.H{"error": "Failed to query analytics", "details": err.Error()})
		return
	}

	c.JSON(http.StatusOK, gin.H{
		"workspace_id":   workspaceID,
		"total_cost_usd": totalCost,
		"total_tokens":   totalTokens,
		"total_requests": requestCount,
	})
}
